#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §45.2's reads, through git and failing closed.
//!
//! **Measured, not assumed** (git 2.43, files backend): `rev-parse --all` exits 0 and omits a ref
//! whose loose file it cannot read, and omits a dangling symbolic ref without a word; `log -g`
//! over an unreadable stash reflog exits 0 with nothing on stdout. A read that believed an exit
//! code would report *no refs, no stashes* about a repository holding both. Every read here
//! therefore refuses — returns `Err` — rather than returns less, and every fixture is built under
//! §45.6's hostile read profile, the config that lies to a naive read.

mod support;

use std::time::Instant;

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{
    GitBackend, HeadState, InterruptedOperation, JobClass, JobContext, RefBackend, StashEntries,
    StatusEntry,
};
use support::git_world::{apply_hostile_read_profile, system_git};
use support::TestRepo;

const fn ctx(cancel: &CancelToken) -> JobContext<'_> {
    JobContext::new(JobClass::Interactive, cancel, None)
}

/// A repository with one commit, under the hostile read profile.
fn hostile_repo() -> TestRepo {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("one");
    apply_hostile_read_profile(&repo);
    repo
}

fn rev(repo: &TestRepo, name: &str) -> String {
    repo.git(&["rev-parse", name]).trim().to_owned()
}

/// **A loose ref git cannot read is omitted by `rev-parse --all`, and the error-strict walk
/// refuses the listing.** The file is `chmod 000`, which is the measured shape.
#[cfg(unix)]
#[test]
fn an_unreadable_loose_ref_is_omitted_by_git_and_refused_by_the_corroboration() {
    use std::os::unix::fs::PermissionsExt as _;

    let repo = hostile_repo();
    repo.git(&["branch", "secret"]);
    let loose = repo.path().join(".git/refs/heads/secret");
    assert!(loose.exists(), "the fixture's branch must be a loose file");
    std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o000)).unwrap();

    // What git itself says, so the test shows the omission it guards against.
    let listed = repo.git(&["rev-parse", "--symbolic-full-name", "--all"]);
    eprintln!(
        "git lists {:?} over an unreadable refs/heads/secret",
        listed.lines().collect::<Vec<_>>()
    );
    assert!(
        !listed.contains("refs/heads/secret"),
        "git no longer omits an unreadable ref; this fixture no longer shows the hazard"
    );

    let cancel = CancelToken::new();
    let outcome = system_git(&repo).enumerate_refs(&repo.handle(), &ctx(&cancel));
    std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o644)).unwrap();
    eprintln!("enumerate_refs over an unreadable loose ref: {outcome:?}");
    assert!(
        outcome.is_err(),
        "an unreadable ref must refuse the listing, never shorten it: {outcome:?}"
    );
}

/// **A garbage loose ref and a dangling symbolic ref are errors, never omissions.** git warns
/// about the first on stderr and says nothing about the second; both still exit 0.
#[test]
fn a_garbage_ref_and_a_dangling_symref_are_errors_not_omissions() {
    let cancel = CancelToken::new();
    let mut shapes = 0;

    let garbage = hostile_repo();
    std::fs::write(
        garbage.path().join(".git/refs/tags/junk"),
        b"not an object id\n",
    )
    .unwrap();
    let outcome = system_git(&garbage).enumerate_refs(&garbage.handle(), &ctx(&cancel));
    eprintln!("garbage loose ref: {outcome:?}");
    assert!(outcome.is_err(), "a garbage ref must refuse the listing");
    shapes += 1;

    let dangling = hostile_repo();
    std::fs::write(
        dangling.path().join(".git/refs/heads/dangling"),
        b"ref: refs/heads/nowhere\n",
    )
    .unwrap();
    let dangled = system_git(&dangling).enumerate_refs(&dangling.handle(), &ctx(&cancel));
    eprintln!("dangling symbolic ref: {dangled:?}");
    assert!(
        dangled.is_err(),
        "a dangling symref must refuse the listing"
    );
    shapes += 1;

    eprintln!("broken shapes refused: {shapes}");
    assert_eq!(shapes, 2);
}

/// Build §45.2 row 1's every namespace, plus the two it excludes.
fn every_namespace_repo() -> (TestRepo, Vec<&'static str>) {
    let repo = hostile_repo();
    let base = rev(&repo, "HEAD");
    repo.write("a.txt", b"two\n");
    repo.commit("two");
    let tip = rev(&repo, "HEAD");
    repo.git(&["tag", "-a", "v1", "-m", "annotated"]);
    repo.git(&["notes", "add", "-m", "a note", "HEAD"]);
    repo.git(&["update-ref", "refs/x/custom", &tip]);
    repo.git(&["update-ref", "refs/original/refs/heads/main", &base]);
    repo.git(&["replace", &base, &tip]);
    repo.git(&["update-ref", "refs/remotes/origin/main", &base]);
    repo.git(&["update-ref", "refs/prefetch/remotes/origin/main", &base]);
    repo.write("a.txt", b"stashed\n");
    repo.git(&["stash", "push", "-q", "-m", "one"]);
    repo.git(&["checkout", "-q", "--detach"]);
    let replace = format!("refs/replace/{base}");
    let wanted = vec![
        "refs/heads/main",
        "refs/tags/v1",
        "refs/notes/commits",
        "refs/x/custom",
        "refs/original/refs/heads/main",
        "refs/stash",
    ];
    // The replace ref's name carries an OID, so it is checked apart from the static list.
    assert!(repo
        .git(&["rev-parse", "--symbolic-full-name", "--all"])
        .contains(&replace));
    (repo, wanted)
}

/// **Every root namespace is listed, and the two tracking namespaces are not** (§45.2 row 1).
/// A detached `HEAD` is its own row (row 2), with its OID.
#[test]
fn every_root_namespace_is_listed_and_tracking_refs_are_not() {
    let (repo, wanted) = every_namespace_repo();
    let cancel = CancelToken::new();
    let listing = system_git(&repo)
        .enumerate_refs(&repo.handle(), &ctx(&cancel))
        .expect("a readable repository lists");
    let names: Vec<&str> = listing.refs.iter().map(|r| r.name.as_str()).collect();
    eprintln!(
        "listed {} refs under {:?}: {names:?}",
        names.len(),
        listing.backend
    );
    assert_eq!(listing.backend, RefBackend::Files);
    for name in &wanted {
        assert!(names.contains(name), "{name} is a root and was not listed");
    }
    assert!(
        names.iter().any(|n| n.starts_with("refs/replace/")),
        "a replace ref is a root"
    );
    for excluded in ["refs/remotes/", "refs/prefetch/"] {
        assert!(
            !names.iter().any(|n| n.starts_with(excluded)),
            "{excluded}* is never a root"
        );
    }
    let tag = listing
        .refs
        .iter()
        .find(|r| r.name == "refs/tags/v1")
        .unwrap();
    assert_eq!(tag.object_type, "tag", "an annotated tag is its tag object");
    assert_eq!(tag.oid, rev(&repo, "refs/tags/v1"));
    match &listing.head {
        HeadState::Detached(oid) => assert_eq!(oid, &rev(&repo, "HEAD")),
        other => panic!("a detached HEAD must list as detached with its OID, got {other:?}"),
    }
    assert!(listing.refs.len() > wanted.len());
}

/// The same listing when the hostile profile arrives through `GIT_CONFIG_GLOBAL` rather than the
/// repository's own config — the other route §45.6 names.
#[test]
fn the_listing_holds_under_a_hostile_global_config() {
    use support::git_world::{child_dir, hostile_read_global, is_child, run_in_child};

    if is_child() {
        let dir = child_dir();
        let repo_path = std::fs::read_to_string(dir.join("repo-path")).unwrap();
        let handle = codotheca_core::git::RepoHandle::resolve(
            std::path::Path::new(repo_path.trim()),
            codotheca_core::git::StoreKey::new("s"),
            codotheca_core::mount::StoreClass::Local,
        )
        .unwrap();
        let exec = codotheca_core::git::GitExec::new(
            support::test_git(),
            codotheca_core::git::ensure_empty_hooks_dir(&dir).unwrap(),
        );
        let git = codotheca_core::git::SystemGit::new(
            std::sync::Arc::new(exec),
            std::sync::Arc::new(codotheca_core::git::GitSlots::for_machine()),
            std::sync::Arc::new(codotheca_core::clock::SystemClock::new()),
        );
        let cancel = CancelToken::new();
        let listing = git.enumerate_refs(&handle, &ctx(&cancel)).expect("lists");
        let stash = git.stash_entries(&handle, &ctx(&cancel)).expect("stash");
        let scan = git.worktree_scan(&handle, &ctx(&cancel)).expect("scan");
        eprintln!(
            "global route: {} refs, stash {stash:?}, {} status entries",
            listing.refs.len(),
            scan.entries.len()
        );
        assert!(listing.refs.iter().any(|r| r.name == "refs/stash"));
        assert!(matches!(stash, StashEntries::Entries(ref e) if e.len() == 1));
        assert!(
            scan.entries
                .iter()
                .any(|e| matches!(e, StatusEntry::Untracked { path } if path == b"loose.txt")),
            "an untracked file must be seen whatever status.showUntrackedFiles says"
        );
        return;
    }

    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("one");
    repo.write("a.txt", b"stashed\n");
    repo.git(&["stash", "push", "-q"]);
    repo.write("loose.txt", b"untracked\n");
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("repo-path"),
        repo.path().to_string_lossy().as_bytes(),
    )
    .unwrap();
    run_in_child(
        "the_listing_holds_under_a_hostile_global_config",
        tmp.path(),
        &hostile_read_global(tmp.path()),
    );
}

/// **Every stash entry, in reflog order, through git** (§45.2 row 3) — and a reflog that is there
/// and cannot be read is `Unreadable`, never *no stash*.
#[test]
fn three_stashes_are_listed_in_reflog_order_and_an_unreadable_reflog_is_unreadable() {
    let repo = hostile_repo();
    for n in 1..=3 {
        repo.write("a.txt", format!("stash {n}\n").as_bytes());
        repo.git(&["stash", "push", "-q", "-m", &format!("s{n}")]);
    }
    let expected: Vec<String> = (0..3)
        .map(|n| rev(&repo, &format!("stash@{{{n}}}")))
        .collect();
    let cancel = CancelToken::new();
    let entries = system_git(&repo)
        .stash_entries(&repo.handle(), &ctx(&cancel))
        .expect("stash reads");
    eprintln!("stash entries: {entries:?}, expected {expected:?}");
    assert_eq!(entries, StashEntries::Entries(expected));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let log = repo.path().join(".git/logs/refs/stash");
        std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o000)).unwrap();
        let unreadable = system_git(&repo).stash_entries(&repo.handle(), &ctx(&cancel));
        std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o644)).unwrap();
        eprintln!("stash over an unreadable reflog: {unreadable:?}");
        assert_eq!(unreadable, Ok(StashEntries::Unreadable));
    }
}

/// A stash whose reflog is gone and whose ref is only packed still counts — as **one**, a floor.
#[test]
fn a_packed_only_stash_counts_one() {
    let repo = hostile_repo();
    repo.write("a.txt", b"stashed\n");
    repo.git(&["stash", "push", "-q"]);
    let tip = rev(&repo, "refs/stash");
    std::fs::remove_file(repo.path().join(".git/logs/refs/stash")).unwrap();
    repo.git(&["pack-refs", "--all"]);
    assert!(
        !repo.path().join(".git/refs/stash").exists(),
        "the stash ref is packed only"
    );
    let cancel = CancelToken::new();
    let entries = system_git(&repo)
        .stash_entries(&repo.handle(), &ctx(&cancel))
        .expect("stash reads");
    eprintln!("packed-only stash: {entries:?}");
    assert_eq!(entries, StashEntries::Entries(vec![tip]));
}

/// **`-unormal` collapses an unignored `node_modules/` to one entry** — `-uall` enumerated all
/// 120,003 of them in D8's measurement and a verdict waited on it. Generated here, not shipped.
#[test]
fn an_unignored_node_modules_is_one_entry_not_a_hundred_thousand() {
    const DIRS: usize = 120;
    const PER_DIR: usize = 1_000;
    let repo = hostile_repo();
    let base = repo.path().join("node_modules");
    let started = Instant::now();
    for d in 0..DIRS {
        let dir = base.join(format!("pkg{d}"));
        std::fs::create_dir_all(&dir).unwrap();
        for f in 0..PER_DIR {
            std::fs::write(dir.join(format!("f{f}.js")), b"x").unwrap();
        }
    }
    eprintln!(
        "generated {} files in {:?}",
        DIRS * PER_DIR,
        started.elapsed()
    );
    let cancel = CancelToken::new();
    let scanned_at = Instant::now();
    let scan = system_git(&repo)
        .worktree_scan(&repo.handle(), &ctx(&cancel))
        .expect("the worktree reads");
    let untracked: Vec<&StatusEntry> = scan
        .entries
        .iter()
        .filter(|e| matches!(e, StatusEntry::Untracked { .. }))
        .collect();
    eprintln!(
        "worktree scan: {} entries, {} untracked, in {:?}: {untracked:?}",
        scan.entries.len(),
        untracked.len(),
        scanned_at.elapsed()
    );
    assert_eq!(
        untracked,
        vec![&StatusEntry::Untracked {
            path: b"node_modules/".to_vec()
        }]
    );
    assert_eq!(scan.index.len(), 1, "the index lists the one tracked file");
}

/// A repository with a "remote" tip and a local-only commit beside it, under the hostile read
/// profile: `(repo, shared base, remote tip, local-only commit)`.
fn diverged() -> (TestRepo, String, String, String) {
    let repo = hostile_repo();
    let shared = rev(&repo, "HEAD");
    repo.git(&["checkout", "-q", "-b", "remote-tip"]);
    repo.write("r.txt", b"remote\n");
    repo.commit("remote");
    let remote = rev(&repo, "HEAD");
    repo.git(&["checkout", "-q", "main"]);
    repo.write("l.txt", b"local\n");
    repo.commit("local only");
    let local = rev(&repo, "HEAD");
    (repo, shared, remote, local)
}

/// Does `local` read as covered by `remote` to git **without** the pins, and to the walk?
fn naive_and_walked(repo: &TestRepo, remote: &str, local: &str) -> (String, bool) {
    let naive = repo.git(&["rev-list", local, &format!("^{remote}")]);
    let cancel = CancelToken::new();
    let walked = system_git(repo)
        .any_uncovered(
            &repo.handle(),
            &[local.to_owned()],
            &[remote.to_owned()],
            &ctx(&cancel),
        )
        .expect("the walk runs");
    (naive.trim().to_owned(), walked)
}

/// **The walk runs with grafts and replace objects both off** (§45.3(a)). A graft file, and
/// separately a replace ref, each make a local-only commit reachable from the "remote" tip.
///
/// Both are proved live: the same question asked of git without the pins reads covered.
#[test]
fn the_walk_ignores_a_graft_and_a_replace_ref() {
    // The graft: the remote tip's parent is rewritten to the local-only commit.
    let (graft, _, graft_remote, graft_local) = diverged();
    std::fs::create_dir_all(graft.path().join(".git/info")).unwrap();
    std::fs::write(
        graft.path().join(".git/info/grafts"),
        format!("{graft_remote} {graft_local}\n"),
    )
    .unwrap();
    let (naive, uncovered) = naive_and_walked(&graft, &graft_remote, &graft_local);
    eprintln!("graft: git without the pins lists {naive:?}; any_uncovered = {uncovered}");
    assert!(
        naive.is_empty(),
        "the graft fixture no longer hides the commit"
    );
    assert!(uncovered, "a graft must not cover a local-only commit");

    // The replace ref: a copy of the remote tip whose parents include the local-only commit.
    let (replaced, shared, remote, local) = diverged();
    let tree = rev(&replaced, &format!("{remote}^{{tree}}"));
    let forged = replaced
        .git(&[
            "commit-tree",
            &tree,
            "-p",
            &local,
            "-p",
            &shared,
            "-m",
            "forged",
        ])
        .trim()
        .to_owned();
    replaced.git(&["replace", &remote, &forged]);
    let (replaced_naive, replaced_uncovered) = naive_and_walked(&replaced, &remote, &local);
    eprintln!(
        "replace: git without the pins lists {replaced_naive:?}; any_uncovered = \
         {replaced_uncovered}"
    );
    assert!(
        replaced_naive.is_empty(),
        "the replace fixture no longer hides the commit"
    );
    assert!(
        replaced_uncovered,
        "a replace ref must not cover a local-only commit"
    );
}

/// The walk answers the plain question too: covered is covered, and uncovered is not.
#[test]
fn the_walk_answers_covered_and_uncovered() {
    let repo = hostile_repo();
    let first = vec![rev(&repo, "HEAD")];
    repo.write("b.txt", b"two\n");
    repo.commit("two");
    let second = vec![rev(&repo, "HEAD")];
    let cancel = CancelToken::new();
    let git = system_git(&repo);
    let h = repo.handle();
    assert!(!git
        .any_uncovered(&h, &first, &second, &ctx(&cancel))
        .unwrap());
    assert!(git
        .any_uncovered(&h, &second, &first, &ctx(&cancel))
        .unwrap());
    assert!(git.any_uncovered(&h, &second, &[], &ctx(&cancel)).unwrap());
    let mut asked = second;
    asked.push("0".repeat(40));
    assert_eq!(
        git.objects_present(&h, &asked, &ctx(&cancel)).unwrap(),
        vec![true, false]
    );
}

/// **An interrupted cherry-pick is found through git** (§45.5): under reftable no
/// `CHERRY_PICK_HEAD` file exists at all, and only `rev-parse --verify` resolves it.
#[test]
fn an_interrupted_cherry_pick_is_found_through_git() {
    let repo = hostile_repo();
    repo.git(&["checkout", "-q", "-b", "side"]);
    repo.write("a.txt", b"side\n");
    repo.commit("side");
    let pick = rev(&repo, "HEAD");
    repo.git(&["checkout", "-q", "main"]);
    repo.write("a.txt", b"main\n");
    repo.commit("main");
    assert!(
        !repo.try_git(&["cherry-pick", &pick]),
        "the fixture's cherry-pick must conflict"
    );
    let cancel = CancelToken::new();
    let found = system_git(&repo)
        .interrupted_ops(&repo.handle(), &ctx(&cancel))
        .expect("reads");
    eprintln!("interrupted operations: {found:?}");
    assert!(found.contains(&InterruptedOperation::CherryPick));

    let clean = hostile_repo();
    let none = system_git(&clean)
        .interrupted_ops(&clean.handle(), &ctx(&cancel))
        .expect("reads");
    assert!(none.is_empty(), "a quiet repository has none: {none:?}");
}

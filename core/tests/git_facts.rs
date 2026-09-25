#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §4.2's four `rev-parse` answers: bare, shallow, and the worktree shape.

mod support;

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{repo_facts, RunLimits};
use support::TestRepo;

#[test]
fn an_ordinary_checkout_is_neither_bare_nor_shallow_nor_linked() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    let f = repo_facts(
        &repo.exec(),
        &repo.handle(),
        RunLimits::none(),
        &CancelToken::new(),
    )
    .unwrap();
    assert!(!f.is_bare);
    assert!(!f.is_shallow);
    assert!(!f.is_linked_worktree());
    assert_eq!(
        f.git_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned()),
        Some(".git".into())
    );
}

#[test]
fn a_bare_repository_reports_itself_as_bare() {
    let repo = TestRepo::init_bare();
    let handle = codotheca_core::git::RepoHandle::bare(
        repo.path(),
        codotheca_core::git::StoreKey::new("s"),
        codotheca_core::mount::StoreClass::Local,
    );
    let f = repo_facts(
        &repo.exec(),
        &handle,
        RunLimits::none(),
        &CancelToken::new(),
    )
    .unwrap();
    assert!(f.is_bare);
    assert!(!f.is_linked_worktree());
}

#[test]
fn a_linked_worktree_has_its_own_git_dir_and_a_shared_common_dir() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    let wt = repo.path().join("linked");
    repo.git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "side",
        wt.to_string_lossy().as_ref(),
    ]);

    let handle = codotheca_core::git::RepoHandle::resolve(
        &wt,
        codotheca_core::git::StoreKey::new("s"),
        codotheca_core::mount::StoreClass::Local,
    )
    .unwrap();
    assert!(
        handle.is_linked_worktree(),
        "the handle resolves it from `commondir` alone"
    );

    let f = repo_facts(
        &repo.exec(),
        &handle,
        RunLimits::none(),
        &CancelToken::new(),
    )
    .unwrap();
    assert!(f.is_linked_worktree(), "git agrees with the handle");
    assert_ne!(f.git_dir, f.common_dir);
}

#[test]
fn a_shallow_clone_is_reported_as_shallow() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    std::fs::write(
        repo.path().join(".git").join("shallow"),
        format!("{}\n", "0".repeat(40)),
    )
    .unwrap();
    let f = repo_facts(
        &repo.exec(),
        &repo.handle(),
        RunLimits::none(),
        &CancelToken::new(),
    )
    .unwrap();
    assert!(f.is_shallow);
}

#[test]
fn a_directory_that_is_not_a_repository_fails_and_does_not_read_as_absent() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
    let handle = codotheca_core::git::RepoHandle::resolve(
        tmp.path(),
        codotheca_core::git::StoreKey::new("s"),
        codotheca_core::mount::StoreClass::Local,
    )
    .unwrap();
    let hooks = codotheca_core::git::ensure_empty_hooks_dir(tmp.path()).unwrap();
    let exec = codotheca_core::git::GitExec::system(hooks);
    let err = repo_facts(&exec, &handle, RunLimits::none(), &CancelToken::new()).unwrap_err();
    assert!(
        !err.implies_absent(),
        "an unreadable repository is not a missing one: {err:?}"
    );
}

// Deviation from the plan, with its regression test. Windows-native, `--absolute-git-dir` comes
// back with forward slashes while `--git-common-dir` comes back relative, so comparing the two
// as strings reported EVERY ordinary checkout as a linked worktree — and §4.2 decides lineage on
// exactly that answer, so plan 08 would treat every repository as the same project elsewhere.
// The plan proposed canonicalising in the assertion; that fixes the test and leaves the product
// wrong. The comparison belongs in `is_linked_worktree`, and the stored values stay as observed.
#[test]
fn two_spellings_of_one_directory_are_not_a_linked_worktree() {
    let tmp = tempfile::tempdir().unwrap();
    let git_dir = tmp.path().join(".git");
    std::fs::create_dir_all(&git_dir).unwrap();
    std::fs::create_dir_all(tmp.path().join("sub")).unwrap();

    let facts = codotheca_core::git::RepoFacts {
        is_bare: false,
        is_shallow: false,
        git_dir,
        // The same directory, spelled the way a relative `--git-common-dir` resolves to.
        common_dir: tmp.path().join("sub").join("..").join(".git"),
    };
    assert_ne!(
        facts.git_dir, facts.common_dir,
        "the fixture is pointless unless the two spellings differ textually"
    );
    assert!(
        !facts.is_linked_worktree(),
        "one directory under two spellings is not two directories"
    );
}

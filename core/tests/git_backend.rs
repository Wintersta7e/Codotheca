#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §15.2's injection seam, exercised through its one real implementation.

mod support;

use std::sync::Arc;
use std::time::Duration;

use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::{Clock, SystemClock};
use codotheca_core::git::{
    ensure_empty_hooks_dir, require_floor, GitBackend, GitError, GitExec, GitSlots, JobClass,
    JobContext, RepoHandle, StatusOptions, StoreKey, SystemGit,
};
use codotheca_core::mount::StoreClass;
use support::TestRepo;

fn backend(repo: &TestRepo) -> Arc<dyn GitBackend> {
    Arc::new(SystemGit::new(
        Arc::new(repo.exec()),
        Arc::new(GitSlots::for_machine()),
        Arc::new(SystemClock::new()) as Arc<dyn Clock>,
    ))
}

fn ctx(cancel: &CancelToken, job: JobClass) -> JobContext<'_> {
    JobContext::new(job, cancel, Some(Duration::from_secs(60)))
}

#[test]
fn the_trait_is_object_safe_and_shareable() {
    fn assert_shareable<T: Send + Sync + ?Sized>() {}
    assert_shareable::<dyn GitBackend>();
}

#[test]
fn the_floor_is_enforced_through_the_trait() {
    let repo = TestRepo::init();
    let cancel = CancelToken::new();
    let v = require_floor(
        backend(&repo).as_ref(),
        &ctx(&cancel, JobClass::Interactive),
    )
    .unwrap();
    assert!(
        (v.major, v.minor) >= (2, 22),
        "the test host needs git >= 2.22, found {}",
        v.raw
    );
    assert!(v.raw.starts_with("git version"));
}

#[test]
fn one_repository_answers_every_method() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"hello\n");
    repo.write("src/main.rs", b"fn main() {}\n");
    repo.commit("first");
    repo.write("a.txt", b"edited\n");
    repo.write("untracked.txt", b"x\n");

    let git = backend(&repo);
    let cancel = CancelToken::new();
    let handle = repo.handle();

    let facts = git
        .repo_facts(&handle, &ctx(&cancel, JobClass::Background))
        .unwrap();
    assert!(!facts.is_bare);

    let state = git
        .ref_state(&handle, &ctx(&cancel, JobClass::Background))
        .unwrap();
    assert_eq!(state.branch.as_deref(), Some("main"));
    assert!(state.head_oid.is_some());

    assert_eq!(
        git.divergence(&handle, &state, &ctx(&cancel, JobClass::Background))
            .unwrap(),
        None
    );

    let st = git
        .worktree_status(
            &handle,
            StatusOptions::full(),
            &ctx(&cancel, JobClass::Interactive),
        )
        .unwrap();
    assert!(st.is_dirty);
    assert_eq!(st.untracked_count, Some(1));

    let inv = git
        .tracked_inventory(&handle, &ctx(&cancel, JobClass::Background))
        .unwrap();
    assert_eq!(inv.tracked_files, 2);

    let roots = git
        .root_commits(&handle, &ctx(&cancel, JobClass::History))
        .unwrap();
    assert_eq!(roots.len(), 1);

    let a = git
        .authorship(&handle, &ctx(&cancel, JobClass::Background))
        .unwrap();
    assert_eq!(a.committers.len(), 1);

    let subjects = git
        .commit_subjects(&handle, 200, &ctx(&cancel, JobClass::History))
        .unwrap();
    assert_eq!(subjects.len(), 1);
    assert_eq!(subjects[0].subject, "first");
}

#[test]
fn a_deadline_of_zero_is_a_budget_failure_not_a_missing_repository() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    let git = backend(&repo);
    let cancel = CancelToken::new();
    let tight = JobContext::new(
        JobClass::Background,
        &cancel,
        Some(Duration::from_millis(0)),
    );
    let err = git
        .worktree_status(&repo.handle(), StatusOptions::full(), &tight)
        .unwrap_err();
    assert!(matches!(err, GitError::Budget { .. }), "{err:?}");
    assert!(!err.implies_absent());
}

/// The zero-budget refusal happens **before** the spawn, and **the missing binary is what proves
/// it**: this backend points at a git that does not exist, so anything reaching a spawn comes back
/// as a spawn failure. Only a refusal taken ahead of the spawn can answer `Budget` here, which is
/// what makes the assertion independent of how fast the machine is.
///
/// The obvious version of this test — a real git and a directory holding no repository — was
/// written first and **proved nothing**: with the refusal disabled it still passed, because the
/// poll loop's own deadline killed a git that took longer than one poll. The control below is the
/// other half: the same call with a real budget does reach the spawn and does not answer `Budget`.
#[test]
fn a_zero_budget_refuses_before_it_spawns_anything() {
    let dir = tempfile::tempdir().unwrap();
    let hooks = ensure_empty_hooks_dir(dir.path()).unwrap();
    let git: Arc<dyn GitBackend> = Arc::new(SystemGit::new(
        Arc::new(GitExec::new(dir.path().join("no-such-git"), hooks)),
        Arc::new(GitSlots::for_machine()),
        Arc::new(SystemClock::new()) as Arc<dyn Clock>,
    ));
    let cancel = CancelToken::new();
    let handle = RepoHandle::bare(dir.path(), StoreKey::new("test-store"), StoreClass::Local);

    let none_left = JobContext::new(JobClass::Background, &cancel, Some(Duration::ZERO));
    let refused = git
        .worktree_status(&handle, StatusOptions::full(), &none_left)
        .unwrap_err();
    assert!(matches!(refused, GitError::Budget { .. }), "{refused:?}");

    let real = ctx(&cancel, JobClass::Background);
    let spawned = git
        .worktree_status(&handle, StatusOptions::full(), &real)
        .unwrap_err();
    assert!(
        !matches!(spawned, GitError::Budget { .. }),
        "the control must fail at the spawn, not on a budget: {spawned:?}"
    );
}

#[test]
fn a_cancelled_context_refuses_before_spawning() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    let git = backend(&repo);
    let cancel = CancelToken::new();
    cancel.cancel();
    let err = git
        .worktree_status(
            &repo.handle(),
            StatusOptions::full(),
            &ctx(&cancel, JobClass::Background),
        )
        .unwrap_err();
    assert_eq!(err, GitError::Cancelled);
}

#[test]
fn an_untrusted_repository_becomes_readable_once_the_handle_is_trusted() {
    // The trust write lives in Codotheca's database; the flag only adds an argument.
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    let git = backend(&repo);
    let cancel = CancelToken::new();
    let trusted = repo.handle().with_trust(true);
    let state = git
        .ref_state(&trusted, &ctx(&cancel, JobClass::Background))
        .unwrap();
    assert_eq!(state.branch.as_deref(), Some("main"));
    let st = git
        .worktree_status(
            &trusted,
            StatusOptions::full(),
            &ctx(&cancel, JobClass::Background),
        )
        .unwrap();
    assert!(!st.is_dirty);
}

/// §1.1's fourth identity fact, which the seam could not read at all until now: the argv builder
/// and the parser both existed and no method ran them, so `IdentityProbe` could not be assembled
/// through `GitBackend`.
///
/// The three properties asserted are the three that would each fail silently: a repository with
/// no remotes is an **answer**, not an error (`git config --get-regexp` exits 1 with empty
/// output and every local-only repository would otherwise be unidentifiable); every remote is
/// returned, not just `origin`, because §1.1's fork rule compares owners across remotes; and a
/// URL containing a newline survives, which is the whole reason the argv asks for `--null`.
#[test]
fn the_seam_reads_every_remote_url_and_no_remotes_is_an_answer() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    let git = backend(&repo);
    let cancel = CancelToken::new();

    assert_eq!(
        git.remote_urls(&repo.handle(), &ctx(&cancel, JobClass::Background))
            .unwrap(),
        Vec::new(),
        "a repository with no remote must answer with an empty list, never an error"
    );

    repo.git(&["remote", "add", "origin", "https://example.invalid/one.git"]);
    repo.git(&[
        "remote",
        "add",
        "upstream",
        "ssh://git@example.invalid/two.git",
    ]);
    // Not reachable through `git remote add`, and exactly what `--null` exists for: a value
    // holding the record separator a line-based parse would split on.
    repo.git(&[
        "config",
        "remote.odd.url",
        "https://example.invalid/a\nb.git",
    ]);

    let mut found = git
        .remote_urls(&repo.handle(), &ctx(&cancel, JobClass::Background))
        .unwrap();
    found.sort();
    assert_eq!(
        found,
        vec![
            (
                "odd".to_owned(),
                "https://example.invalid/a\nb.git".to_owned()
            ),
            (
                "origin".to_owned(),
                "https://example.invalid/one.git".to_owned()
            ),
            (
                "upstream".to_owned(),
                "ssh://git@example.invalid/two.git".to_owned()
            ),
        ]
    );
}

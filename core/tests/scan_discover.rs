//! Compiled only under `testkit`: these link `codotheca_core::testing`, which the feature gates.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §4.2 — what a directory is.
//!
//! **Deviation from plan 07 Task 5, recorded here because the tests are what pin it.** The plan
//! calls `GitBackend::run(GitRequest { args: ["rev-parse", …] })`. Plan 05 shipped `GitBackend`
//! as one method per operation with no argv at the boundary, and `repo_facts` is precisely
//! §4.2's four `rev-parse` answers — `--is-bare-repository --is-shallow-repository
//! --absolute-git-dir --git-common-dir` — in one invocation, under the §3.4 slot caps. So
//! discovery goes through `repo_facts`, and the fake is plan 06's `FakeGitBackend` (R29) rather
//! than a second in-crate double.

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{GitError, RepoFacts, StoreKey};
use codotheca_core::mount::StoreClass;
use codotheca_core::scan::discover::{classify_dir, parse_gitdir_pointer, ProbeCtx, RepoKind};
use codotheca_core::scan::{ScanProblemKind, WalkEvent, WalkOptions};
use codotheca_core::testing::{FakeGitBackend, GitReply};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

fn opts() -> WalkOptions {
    WalkOptions {
        bare_candidates: false,
        ..WalkOptions::default()
    }
}

fn probe<'a>(git: &'a FakeGitBackend, cancel: &'a CancelToken) -> ProbeCtx<'a> {
    ProbeCtx::new(git, StoreKey::new("store-a"), StoreClass::Local, cancel)
}

fn facts(is_bare: bool, git_dir: &str, common_dir: &str) -> RepoFacts {
    RepoFacts {
        is_bare,
        is_shallow: false,
        git_dir: PathBuf::from(git_dir),
        common_dir: PathBuf::from(common_dir),
    }
}

fn bare_opts() -> WalkOptions {
    WalkOptions {
        bare_candidates: true,
        ..WalkOptions::default()
    }
}

fn make_bare_shaped(base: &Path) {
    std::fs::write(base.join("HEAD"), b"ref: refs/heads/main\n").unwrap();
    std::fs::create_dir_all(base.join("objects")).unwrap();
    std::fs::create_dir_all(base.join("refs")).unwrap();
}

#[test]
fn a_dot_git_directory_is_a_working_tree_and_costs_no_git_call() {
    let base = tempfile::tempdir().unwrap();
    let dir = base.path();
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let git = FakeGitBackend::new();
    let cancel = CancelToken::new();
    let found = classify_dir(dir, &opts(), &probe(&git, &cancel), &|_| {}).unwrap();
    assert_eq!(found.kind, RepoKind::WorkTree);
    assert_eq!(found.path, dir);
    assert_eq!(found.git_dir, dir.join(".git"));
    assert_eq!(found.common_dir, dir.join(".git"));
    assert!(git.calls().is_empty(), "the filesystem alone answers this");
}

#[test]
fn a_plain_directory_is_nothing() {
    let base = tempfile::tempdir().unwrap();
    let git = FakeGitBackend::new();
    let cancel = CancelToken::new();
    assert!(classify_dir(base.path(), &opts(), &probe(&git, &cancel), &|_| {}).is_none());
    assert!(git.calls().is_empty());
}

#[test]
fn the_pointer_parser_takes_the_gitdir_line_and_trims_it() {
    assert_eq!(
        parse_gitdir_pointer(b"gitdir: /x/.git/worktrees/w\n"),
        Some(PathBuf::from("/x/.git/worktrees/w"))
    );
    assert_eq!(
        parse_gitdir_pointer(b"gitdir:/x/.git\n"),
        Some(PathBuf::from("/x/.git"))
    );
    assert!(parse_gitdir_pointer(b"not a pointer\n").is_none());
    assert!(parse_gitdir_pointer(b"gitdir:\n").is_none());
}

#[test]
fn a_differing_common_dir_is_a_linked_worktree() {
    let base = tempfile::tempdir().unwrap();
    let dir = base.path();
    std::fs::write(dir.join(".git"), b"gitdir: /x/.git/worktrees/w\n").unwrap();
    let git = FakeGitBackend::new();
    git.always_repo_facts(GitReply::Ok(facts(false, "/x/.git/worktrees/w", "/x/.git")));
    let cancel = CancelToken::new();
    let found = classify_dir(dir, &opts(), &probe(&git, &cancel), &|_| {}).unwrap();
    assert_eq!(found.kind, RepoKind::LinkedWorktree);
    assert_eq!(found.git_dir, Path::new("/x/.git/worktrees/w"));
    assert_eq!(found.common_dir, Path::new("/x/.git"));
}

#[test]
fn an_equal_common_dir_is_its_own_repository_not_a_worktree() {
    let base = tempfile::tempdir().unwrap();
    let dir = base.path();
    std::fs::write(dir.join(".git"), b"gitdir: /s/.git/modules/lib\n").unwrap();
    let git = FakeGitBackend::new();
    git.always_repo_facts(GitReply::Ok(facts(
        false,
        "/s/.git/modules/lib",
        "/s/.git/modules/lib",
    )));
    let cancel = CancelToken::new();
    let found = classify_dir(dir, &opts(), &probe(&git, &cancel), &|_| {}).unwrap();
    assert_eq!(found.kind, RepoKind::SeparateGitDir);
    assert_eq!(found.common_dir, Path::new("/s/.git/modules/lib"));
}

/// Criterion 5: an unreadable repository indexes rather than vanishing. The textual shape of the
/// pointer is the fallback — nothing else in git's layout uses a `worktrees` component.
///
/// The pointer is built from the temp directory rather than written as `/x/.git/…`: a path with
/// a leading separator and no drive prefix is absolute on Unix and **not** on Windows, so the
/// literal fixture asserted one thing on one host and another on the other.
#[test]
fn a_failed_probe_still_indexes_the_repository_and_explains_itself() {
    let base = tempfile::tempdir().unwrap();
    let dir = base.path();
    let store = dir.join("store");
    std::fs::write(
        dir.join(".git"),
        format!("gitdir: {}\n", store.join(".git/worktrees/w").display()).as_bytes(),
    )
    .unwrap();
    let git = FakeGitBackend::new();
    git.always_repo_facts(GitReply::Err(GitError::Unreadable {
        detail: "dubious ownership".to_owned(),
    }));
    let cancel = CancelToken::new();
    let problems = Mutex::new(Vec::new());
    let found = classify_dir(dir, &opts(), &probe(&git, &cancel), &|event| {
        if let WalkEvent::Problem(p) = event {
            problems.lock().unwrap().push(p);
        }
    })
    .unwrap();
    assert_eq!(found.kind, RepoKind::LinkedWorktree);
    assert_eq!(found.common_dir, store.join(".git"));
    let problems = problems.into_inner().unwrap();
    assert_eq!(problems.len(), 1);
    assert_eq!(
        problems.first().map(|p| p.kind),
        Some(ScanProblemKind::UnreadableRepo)
    );
}

#[test]
fn a_failed_probe_on_a_plain_pointer_is_its_own_repository() {
    let base = tempfile::tempdir().unwrap();
    let dir = base.path();
    std::fs::write(dir.join(".git"), b"gitdir: /s/.git/modules/lib\n").unwrap();
    let git = FakeGitBackend::new();
    git.always_repo_facts(GitReply::Err(GitError::Unreadable {
        detail: "no".to_owned(),
    }));
    let cancel = CancelToken::new();
    let found = classify_dir(dir, &opts(), &probe(&git, &cancel), &|_| {}).unwrap();
    assert_eq!(found.kind, RepoKind::SeparateGitDir);
}

/// A relative `gitdir:` is resolved against the directory holding the `.git` file, or the
/// fallback would name a path that does not exist.
#[test]
fn a_relative_pointer_is_resolved_against_the_worktree() {
    let base = tempfile::tempdir().unwrap();
    let dir = base.path();
    std::fs::write(dir.join(".git"), b"gitdir: ../store/.git/worktrees/w\n").unwrap();
    let git = FakeGitBackend::new();
    git.always_repo_facts(GitReply::Err(GitError::Unreadable {
        detail: "no".to_owned(),
    }));
    let cancel = CancelToken::new();
    let found = classify_dir(dir, &opts(), &probe(&git, &cancel), &|_| {}).unwrap();
    assert_eq!(found.kind, RepoKind::LinkedWorktree);
    assert_eq!(found.git_dir, dir.join("../store/.git/worktrees/w"));
    assert_eq!(found.common_dir, dir.join("../store/.git"));
}

#[test]
fn an_oversized_dot_git_file_is_reported_and_not_indexed() {
    let base = tempfile::tempdir().unwrap();
    let dir = base.path();
    std::fs::write(dir.join(".git"), vec![b'x'; 8 * 1024]).unwrap();
    let git = FakeGitBackend::new();
    let cancel = CancelToken::new();
    let problems = Mutex::new(Vec::new());
    let found = classify_dir(dir, &opts(), &probe(&git, &cancel), &|event| {
        if let WalkEvent::Problem(p) = event {
            problems.lock().unwrap().push(p);
        }
    });
    assert!(found.is_none());
    assert_eq!(problems.into_inner().unwrap().len(), 1);
    assert!(git.calls().is_empty());
}

#[test]
fn a_dot_git_file_with_no_gitdir_line_is_reported_and_not_indexed() {
    let base = tempfile::tempdir().unwrap();
    let dir = base.path();
    std::fs::write(dir.join(".git"), b"something else entirely\n").unwrap();
    let git = FakeGitBackend::new();
    let cancel = CancelToken::new();
    let problems = Mutex::new(Vec::new());
    assert!(classify_dir(dir, &opts(), &probe(&git, &cancel), &|event| {
        if let WalkEvent::Problem(p) = event {
            problems.lock().unwrap().push(p);
        }
    })
    .is_none());
    assert_eq!(problems.into_inner().unwrap().len(), 1);
    assert!(git.calls().is_empty());
}

// ---- Task 6: bare repositories ------------------------------------------------------------

#[test]
fn the_triple_plus_a_true_answer_is_a_bare_repository() {
    let base = tempfile::tempdir().unwrap();
    let dir = base.path();
    make_bare_shaped(dir);
    let git = FakeGitBackend::new();
    git.always_repo_facts(GitReply::Ok(facts(
        true,
        &dir.display().to_string(),
        &dir.display().to_string(),
    )));
    let cancel = CancelToken::new();
    let found = classify_dir(dir, &bare_opts(), &probe(&git, &cancel), &|_| {}).unwrap();
    assert_eq!(found.kind, RepoKind::Bare);
    assert_eq!(found.git_dir, dir);
    assert_eq!(found.common_dir, dir);
    assert!(!found.kind.has_worktree());
}

#[test]
fn a_false_answer_is_not_a_repository() {
    let base = tempfile::tempdir().unwrap();
    let dir = base.path();
    make_bare_shaped(dir);
    let git = FakeGitBackend::new();
    git.always_repo_facts(GitReply::Ok(facts(false, "/x", "/x")));
    let cancel = CancelToken::new();
    assert!(classify_dir(dir, &bare_opts(), &probe(&git, &cancel), &|_| {}).is_none());
}

#[test]
fn an_incomplete_triple_never_reaches_git() {
    let base = tempfile::tempdir().unwrap();
    let dir = base.path();
    std::fs::write(dir.join("HEAD"), b"ref: refs/heads/main\n").unwrap();
    std::fs::create_dir_all(dir.join("objects")).unwrap();
    // no refs/
    let git = FakeGitBackend::new();
    git.always_repo_facts(GitReply::Ok(facts(true, "/x", "/x")));
    let cancel = CancelToken::new();
    assert!(classify_dir(dir, &bare_opts(), &probe(&git, &cancel), &|_| {}).is_none());
    assert!(git.calls().is_empty());
}

#[test]
fn candidates_are_not_tested_where_the_root_did_not_enable_them() {
    let base = tempfile::tempdir().unwrap();
    let dir = base.path();
    make_bare_shaped(dir);
    let git = FakeGitBackend::new();
    git.always_repo_facts(GitReply::Ok(facts(true, "/x", "/x")));
    let cancel = CancelToken::new();
    assert!(classify_dir(dir, &opts(), &probe(&git, &cancel), &|_| {}).is_none());
    assert!(git.calls().is_empty());
}

/// The hazard, not the fix: every `.git` directory has the bare shape, so the only thing keeping
/// one repository from becoming two is the walk's `.git` guard, asserted in `scan_walk.rs`.
#[test]
fn a_git_directory_has_the_bare_shape_which_is_why_the_walk_must_never_enter_one() {
    let base = tempfile::tempdir().unwrap();
    let dot = base.path().join(".git");
    std::fs::create_dir_all(&dot).unwrap();
    make_bare_shaped(&dot);
    let git = FakeGitBackend::new();
    git.always_repo_facts(GitReply::Ok(facts(
        true,
        &dot.display().to_string(),
        &dot.display().to_string(),
    )));
    let cancel = CancelToken::new();
    assert_eq!(
        classify_dir(&dot, &bare_opts(), &probe(&git, &cancel), &|_| {}).map(|c| c.kind),
        Some(RepoKind::Bare)
    );
}

#[test]
fn a_failed_probe_on_a_bare_shaped_directory_is_reported_and_not_indexed() {
    let base = tempfile::tempdir().unwrap();
    let dir = base.path();
    make_bare_shaped(dir);
    let git = FakeGitBackend::new();
    git.always_repo_facts(GitReply::Err(GitError::Unreadable {
        detail: "no".to_owned(),
    }));
    let cancel = CancelToken::new();
    let problems = Mutex::new(Vec::new());
    assert!(
        classify_dir(dir, &bare_opts(), &probe(&git, &cancel), &|event| {
            if let WalkEvent::Problem(p) = event {
                problems.lock().unwrap().push(p);
            }
        })
        .is_none()
    );
    assert_eq!(problems.into_inner().unwrap().len(), 1);
}

/// R7: `repo_kind` is a TEXT column and also crosses the protocol, so `as_str` needs an inverse
/// and both directions have to agree with serde. Every variant, or a kind added later gets a
/// writer and no reader — which is how `location` rows become unreadable.
#[test]
fn every_repo_kind_round_trips_through_text_and_through_serde() {
    for kind in [
        RepoKind::WorkTree,
        RepoKind::LinkedWorktree,
        RepoKind::SeparateGitDir,
        RepoKind::Bare,
    ] {
        assert_eq!(RepoKind::from_str(kind.as_str()), Some(kind));
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(
            json,
            format!("\"{}\"", kind.as_str()),
            "serde must match as_str"
        );
        assert_eq!(serde_json::from_str::<RepoKind>(&json).unwrap(), kind);
    }
    assert_eq!(RepoKind::from_str("submodule"), None);
}

/// The DDL's CHECK constraint and `as_str` are one vocabulary stated twice (R26's shape). This
/// reads the migration and asserts every value the code can write is one the column accepts.
#[test]
fn every_repo_kind_is_a_value_the_location_check_constraint_accepts() {
    let sql = include_str!("../migrations/0002_locations_and_roots.sql");
    for kind in [
        RepoKind::WorkTree,
        RepoKind::LinkedWorktree,
        RepoKind::SeparateGitDir,
        RepoKind::Bare,
    ] {
        assert!(
            sql.contains(&format!("'{}'", kind.as_str())),
            "repo_kind '{}' is emitted by the core and rejected by the DDL",
            kind.as_str()
        );
    }
}

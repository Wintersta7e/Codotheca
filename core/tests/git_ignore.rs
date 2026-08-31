#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! `check_ignore` against real git. §9's activity scope is decided by the repository's own
//! rules, so the only test that proves it is one that runs them.

mod support;

use std::path::PathBuf;

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{check_ignore, RunLimits, CHECK_IGNORE_BATCH};

use support::TestRepo;

fn rel(paths: &[&str]) -> Vec<PathBuf> {
    paths.iter().map(PathBuf::from).collect()
}

fn verdicts(repo: &TestRepo, paths: &[&str]) -> Vec<bool> {
    check_ignore(
        &repo.exec(),
        &repo.handle(),
        RunLimits::none(),
        &CancelToken::new(),
        &rel(paths),
    )
    .unwrap()
}

#[test]
fn a_build_directory_is_ignored_and_a_source_file_is_not() {
    let repo = TestRepo::init();
    repo.write(".gitignore", b"dist/\n");
    repo.write("src/main.rs", b"fn main() {}\n");
    repo.write("dist/app.js", b"//\n");

    assert_eq!(
        verdicts(&repo, &["dist/app.js", "src/main.rs"]),
        vec![true, false],
        "one verdict per input, in the order given"
    );
}

#[test]
fn a_repository_with_no_ignore_rules_reports_nothing_ignored_rather_than_failing() {
    // `check-ignore` exits 1 here, which `run` alone would raise as a GitError and which both
    // possible guesses get wrong: "all ignored" would credit nothing in such a repository, and
    // "all in scope" would credit a dev server in one that does have rules.
    let repo = TestRepo::init();
    repo.write("src/main.rs", b"fn main() {}\n");

    assert_eq!(verdicts(&repo, &["src/main.rs", "a.txt"]), vec![false; 2]);
}

#[test]
fn a_force_added_tracked_file_inside_an_ignored_directory_stays_in_scope() {
    // The reason the verdict is keyed on the full path and never on its parent directory:
    // `check-ignore` consults the index, and a directory is not a tracked path.
    let repo = TestRepo::init();
    repo.write(".gitignore", b"dist/\n");
    repo.write("dist/keep.js", b"//\n");
    repo.write("dist/other.js", b"//\n");
    repo.git(&["add", "-f", "dist/keep.js"]);

    assert_eq!(
        verdicts(&repo, &["dist/keep.js", "dist/other.js"]),
        vec![false, true],
        "the tracked file is in scope even though its directory is ignored"
    );
}

#[test]
fn a_batch_larger_than_one_argv_call_is_answered_in_order() {
    // The batching is invisible from outside: the verdicts must line up with the inputs across
    // the chunk boundary, which is where an off-by-one would hide.
    let repo = TestRepo::init();
    repo.write(".gitignore", b"*.log\n");

    let names: Vec<String> = (0..CHECK_IGNORE_BATCH * 2 + 5)
        .map(|n| {
            if n % 3 == 0 {
                format!("f{n}.log")
            } else {
                format!("f{n}.rs")
            }
        })
        .collect();
    let paths: Vec<PathBuf> = names.iter().map(PathBuf::from).collect();
    let got = check_ignore(
        &repo.exec(),
        &repo.handle(),
        RunLimits::none(),
        &CancelToken::new(),
        &paths,
    )
    .unwrap();

    assert_eq!(got.len(), paths.len());
    let want: Vec<bool> = (0..paths.len()).map(|n| n % 3 == 0).collect();
    assert_eq!(got, want, "every verdict still belongs to its own path");
}

#[test]
fn an_empty_input_asks_git_nothing() {
    let repo = TestRepo::init();
    assert!(verdicts(&repo, &[]).is_empty());
}

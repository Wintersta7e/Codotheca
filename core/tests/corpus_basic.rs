//! Compiled only under `testkit`: these link `codotheca_core::corpus`, which the feature
//! gates. Without the gate a bare `cargo test` fails to compile rather than skipping.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::corpus::fixtures as ids;
use codotheca_core::corpus::{generate, CorpusManifest, CorpusOptions};

fn build(names: &[&str], scratch: &str) -> CorpusManifest {
    let dir = std::env::temp_dir().join(format!("codotheca-corpus-test-{scratch}"));
    let mut options = CorpusOptions::new(dir);
    options.only = Some(names.iter().map(|n| (*n).to_owned()).collect());
    options.untracked_files = 25;
    generate(&options).unwrap()
}

#[test]
fn a_zero_commit_repository_has_a_git_directory_and_no_head() {
    let m = build(&[ids::ZERO_COMMIT], "zero");
    let f = m.require(ids::ZERO_COMMIT).unwrap();
    assert!(f.path.join(".git").is_dir());
    assert_eq!(f.expect.head_oid, None);
    assert!(f.expect.root_oids.is_empty());
}

#[test]
fn a_bare_repository_has_no_dot_git_entry_but_the_three_marker_children() {
    let m = build(&[ids::BARE], "bare");
    let f = m.require(ids::BARE).unwrap();
    assert!(
        !f.path.join(".git").exists(),
        "a bare repository is why the .git detector missed it"
    );
    assert!(f.path.join("HEAD").is_file());
    assert!(f.path.join("objects").is_dir());
    assert!(f.path.join("refs").is_dir());
    assert!(f.expect.bare);
    let git = m.git().unwrap();
    assert_eq!(
        git.run(&f.path, 0, &["rev-parse", "--is-bare-repository"])
            .unwrap(),
        "true"
    );
}

#[test]
fn a_shallow_clone_reports_shallow_and_carries_one_commit() {
    let m = build(&[ids::SHALLOW], "shallow");
    let f = m.require(ids::SHALLOW).unwrap();
    assert!(f.expect.shallow);
    assert!(f.path.join(".git").join("shallow").is_file());
    let git = m.git().unwrap();
    assert_eq!(
        git.run(&f.path, 0, &["rev-parse", "--is-shallow-repository"])
            .unwrap(),
        "true"
    );
    assert_eq!(
        git.run(&f.path, 0, &["rev-list", "--count", "HEAD"])
            .unwrap(),
        "1"
    );
}

#[test]
fn a_multi_root_repository_has_two_root_commits_reachable_from_head() {
    let m = build(&[ids::MULTI_ROOT], "multiroot");
    let f = m.require(ids::MULTI_ROOT).unwrap();
    assert_eq!(f.expect.root_oids.len(), 2);
    let git = m.git().unwrap();
    let roots = git
        .run(&f.path, 0, &["rev-list", "--max-parents=0", "HEAD"])
        .unwrap();
    assert_eq!(roots.lines().count(), 2);
}

#[test]
fn the_future_dated_commit_really_is_in_the_future() {
    let m = build(&[ids::FUTURE_DATED], "future");
    let f = m.require(ids::FUTURE_DATED).unwrap();
    let git = m.git().unwrap();
    let at = git.run(&f.path, 0, &["log", "-1", "--format=%ct"]).unwrap();
    assert_eq!(at, "4102444800");
}

#[test]
fn the_locked_repository_holds_an_index_lock_on_disk() {
    let m = build(&[ids::INDEX_LOCK_HELD], "locked");
    let f = m.require(ids::INDEX_LOCK_HELD).unwrap();
    assert!(f.expect.index_lock_held);
    assert!(f.path.join(".git").join("index.lock").is_file());
}

#[test]
fn the_huge_untracked_area_is_untracked_and_not_ignored() {
    let m = build(&[ids::HUGE_UNTRACKED], "untracked");
    let f = m.require(ids::HUGE_UNTRACKED).unwrap();
    assert_eq!(f.expect.untracked_files, 25);
    let git = m.git().unwrap();
    // `-uall`, because plain `--porcelain` collapses an untracked directory into one `?? dir/`
    // entry: without it this asserts how git summarises, not what the fixture contains.
    let status = git
        .run(&f.path, 0, &["status", "--porcelain", "-uall"])
        .unwrap();
    assert_eq!(status.lines().filter(|l| l.starts_with("?? ")).count(), 25);
}

#[test]
fn every_basic_fixture_is_listed_in_the_build_order() {
    for name in [
        ids::UPSTREAM,
        ids::OTHER_UPSTREAM,
        ids::ZERO_COMMIT,
        ids::BARE,
        ids::SHALLOW,
        ids::MULTI_ROOT,
        ids::FUTURE_DATED,
        ids::INDEX_LOCK_HELD,
        ids::HUGE_UNTRACKED,
    ] {
        assert!(ids::ORDER.contains(&name), "{name} is missing from ORDER");
    }
}

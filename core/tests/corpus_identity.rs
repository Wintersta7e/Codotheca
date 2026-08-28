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
use codotheca_core::corpus::{generate, CorpusManifest, CorpusOptions, VOLUME_A, VOLUME_B};

fn build(names: &[&str], scratch: &str) -> CorpusManifest {
    let dir = std::env::temp_dir().join(format!("codotheca-corpus-test-{scratch}"));
    let mut options = CorpusOptions::new(dir);
    options.only = Some(names.iter().map(|n| (*n).to_owned()).collect());
    generate(&options).unwrap()
}

#[test]
fn a_fork_shares_the_upstreams_root_commit_and_carries_a_different_remote() {
    let m = build(&[ids::UPSTREAM, ids::FORK], "fork");
    let up = m.require(ids::UPSTREAM).unwrap();
    let fork = m.require(ids::FORK).unwrap();
    assert_eq!(up.expect.root_oids, fork.expect.root_oids);
    assert_ne!(up.expect.head_oid, fork.expect.head_oid);
    assert_ne!(up.expect.origin_url, fork.expect.origin_url);
    assert!(fork.expect.origin_url.is_some());
}

#[test]
fn the_fork_is_ahead_of_its_remote_and_has_never_fetched() {
    // Criterion 63's fixture: ahead > 0 with no FETCH_HEAD, so no BEHIND chip and no age.
    let m = build(&[ids::UPSTREAM, ids::FORK], "forkahead");
    let fork = m.require(ids::FORK).unwrap();
    assert!(!fork.path.join(".git").join("FETCH_HEAD").exists());
    let git = m.git().unwrap();
    let ahead = git
        .run(&fork.path, 0, &["rev-list", "--count", "origin/main..HEAD"])
        .unwrap();
    assert_eq!(ahead, "1");
}

#[test]
fn two_copies_of_one_repository_sit_on_two_volumes_with_one_remote() {
    let m = build(&[ids::UPSTREAM, ids::COPY_ONE, ids::COPY_TWO], "copies");
    let one = m.require(ids::COPY_ONE).unwrap();
    let two = m.require(ids::COPY_TWO).unwrap();
    assert_eq!(one.volume, VOLUME_A);
    assert_eq!(two.volume, VOLUME_B);
    assert_eq!(one.expect.origin_url, two.expect.origin_url);
    assert_eq!(one.expect.root_oids, two.expect.root_oids);
    assert_eq!(one.expect.head_oid, two.expect.head_oid);
}

#[test]
fn the_ambiguous_clone_has_two_candidate_lineages_and_no_remote() {
    let m = build(
        &[ids::UPSTREAM, ids::OTHER_UPSTREAM, ids::AMBIGUOUS_LINEAGE],
        "ambig",
    );
    let up = m.require(ids::UPSTREAM).unwrap();
    let other = m.require(ids::OTHER_UPSTREAM).unwrap();
    let amb = m.require(ids::AMBIGUOUS_LINEAGE).unwrap();
    assert_eq!(amb.expect.origin_url, None);
    assert_eq!(amb.expect.root_oids.len(), 2);
    for root in up
        .expect
        .root_oids
        .iter()
        .chain(other.expect.root_oids.iter())
    {
        assert!(
            amb.expect.root_oids.contains(root),
            "missing candidate root {root}"
        );
    }
    let git = m.git().unwrap();
    assert_eq!(git.run(&amb.path, 0, &["remote"]).unwrap(), "");
}

#[test]
fn a_repository_inside_a_repository_is_not_a_submodule() {
    let m = build(
        &[ids::REPO_INSIDE_REPO_OUTER, ids::REPO_INSIDE_REPO_INNER],
        "nested",
    );
    let outer = m.require(ids::REPO_INSIDE_REPO_OUTER).unwrap();
    let inner = m.require(ids::REPO_INSIDE_REPO_INNER).unwrap();
    assert!(inner.path.starts_with(&outer.path));
    assert!(
        inner.path.join(".git").is_dir(),
        "a plain nested repo has a .git directory"
    );
    assert!(!outer.path.join(".gitmodules").exists());
    assert_ne!(outer.expect.root_oids, inner.expect.root_oids);
}

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

fn build(scratch: &str, large: bool, commits: u32) -> CorpusManifest {
    let dir = std::env::temp_dir().join(format!("codotheca-corpus-test-{scratch}"));
    let mut options = CorpusOptions::new(dir);
    options.only = Some(vec![ids::DEEP_HISTORY.to_owned()]);
    options.large = large;
    options.deep_history_commits = commits;
    generate(&options).unwrap()
}

#[test]
fn without_the_large_flag_the_fixture_is_absent_and_says_why() {
    let m = build("deep-off", false, 200);
    let f = m.fixture(ids::DEEP_HISTORY).unwrap();
    assert!(!f.materialised);
    assert_eq!(
        f.skip_reason.as_deref(),
        Some("not requested: pass --large")
    );
    assert!(m.require(ids::DEEP_HISTORY).is_err());
}

#[test]
fn with_the_large_flag_the_history_is_exactly_as_deep_as_asked() {
    let m = build("deep-on", true, 200);
    let f = m.require(ids::DEEP_HISTORY).unwrap();
    assert_eq!(f.expect.history_depth, Some(200));
    let git = m.git().unwrap();
    assert_eq!(
        git.run(&f.path, 0, &["rev-list", "--count", "HEAD"])
            .unwrap(),
        "200"
    );
    assert_eq!(f.expect.root_oids.len(), 1);
    // The worktree is checked out, so the walk finds a real repository and not a husk.
    assert!(f.path.join("counter.txt").is_file());
}

#[test]
fn the_deep_history_is_deterministic_across_runs() {
    let first = build("deep-det-a", true, 50);
    let second = build("deep-det-b", true, 50);
    assert_eq!(
        first.require(ids::DEEP_HISTORY).unwrap().expect.head_oid,
        second.require(ids::DEEP_HISTORY).unwrap().expect.head_oid,
    );
}

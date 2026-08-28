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
    generate(&options).unwrap()
}

#[test]
fn the_non_utf8_path_is_in_the_tree_where_both_platforms_can_hold_it() {
    let m = build(&[ids::NON_UTF8_PATH], "nonutf8");
    let f = m.require(ids::NON_UTF8_PATH).unwrap();
    let git = m.git().unwrap();
    // -z keeps the raw bytes; without it git quotes them and the test would assert quoting.
    let listed = git
        .run_bytes(&f.path, 0, &["ls-tree", "-z", "--name-only", "HEAD"], &[])
        .unwrap();
    assert!(
        listed.contains(&0xffu8),
        "the invalid byte must survive into the tree"
    );
    assert!(String::from_utf8(listed).is_err());
}

#[test]
fn the_long_path_exceeds_the_windows_limit_and_is_tracked() {
    let m = build(&[ids::LONG_PATH], "longpath");
    let f = m.require(ids::LONG_PATH).unwrap();
    assert!(f.expect.requires_long_paths);
    let git = m.git().unwrap();
    let tracked = git.run(&f.path, 0, &["ls-files"]).unwrap();
    let deepest = tracked.lines().max_by_key(|l| l.len()).unwrap();
    assert!(
        f.path.display().to_string().len() + deepest.len() > 260,
        "the fixture must actually cross the limit it exists to test"
    );
}

#[test]
fn the_symlink_cycle_points_at_its_own_ancestor_or_says_why_not() {
    let m = build(&[ids::SYMLINK_CYCLE], "symlink");
    let f = m.fixture(ids::SYMLINK_CYCLE).unwrap();
    if f.materialised {
        let link = f.path.join("a").join("loop");
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(std::fs::canonicalize(&link).unwrap().ends_with("a"));
    } else {
        assert_eq!(
            f.skip_reason.as_deref(),
            Some("symlink creation refused by the platform")
        );
        assert!(
            m.require(ids::SYMLINK_CYCLE).is_err(),
            "a skip must never read as a pass"
        );
    }
}

#[test]
fn the_untrusted_repository_exists_and_states_that_its_refusal_is_injected() {
    let m = build(&[ids::DUBIOUS_OWNERSHIP], "dubious");
    let f = m.require(ids::DUBIOUS_OWNERSHIP).unwrap();
    assert!(f.path.join(".git").is_dir());
    assert!(f
        .expect
        .notes
        .as_deref()
        .unwrap_or_default()
        .contains("injected"));
}

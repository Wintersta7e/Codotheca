//! Compiled only under `testkit`: these link `codotheca_core::corpus`, which the feature
//! gates. Without the gate a bare `cargo test` fails to compile rather than skipping.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};

use codotheca_core::corpus::fixtures as ids;
use codotheca_core::corpus::{generate, CorpusManifest, CorpusOptions};

fn build(names: &[&str], scratch: &str) -> CorpusManifest {
    let dir = std::env::temp_dir().join(format!("codotheca-corpus-test-{scratch}"));
    let mut options = CorpusOptions::new(dir);
    options.only = Some(names.iter().map(|n| (*n).to_owned()).collect());
    generate(&options).unwrap()
}

/// `rev-parse --git-common-dir` may answer relatively; resolve it against the repository.
fn common_dir(m: &CorpusManifest, path: &Path) -> PathBuf {
    let raw = m
        .git()
        .unwrap()
        .run(path, 0, &["rev-parse", "--git-common-dir"])
        .unwrap();
    let joined = if Path::new(&raw).is_absolute() {
        PathBuf::from(&raw)
    } else {
        path.join(&raw)
    };
    std::fs::canonicalize(joined).unwrap()
}

#[test]
fn a_linked_worktree_has_a_git_file_whose_common_dir_is_the_parents() {
    let m = build(&[ids::WORKTREE_PARENT, ids::LINKED_WORKTREE], "worktree");
    let parent = m.require(ids::WORKTREE_PARENT).unwrap();
    let linked = m.require(ids::LINKED_WORKTREE).unwrap();

    let marker = linked.path.join(".git");
    assert!(
        marker.is_file(),
        "a linked worktree marks itself with a .git file, not a directory"
    );
    let text = std::fs::read_to_string(&marker).unwrap();
    assert!(text.starts_with("gitdir:"));
    assert!(text.contains("worktrees"), "got {text}");

    assert_eq!(common_dir(&m, &linked.path), common_dir(&m, &parent.path));
    assert_eq!(
        linked.expect.parent_fixture.as_deref(),
        Some(ids::WORKTREE_PARENT)
    );
    // Same project: the two share a root commit because they are one repository.
    assert_eq!(linked.expect.root_oids, parent.expect.root_oids);
}

#[test]
fn a_submodule_has_a_git_file_whose_common_dir_is_its_own_repository() {
    let m = build(&[ids::SUBMODULE_PARENT], "submodule");
    let parent = m.require(ids::SUBMODULE_PARENT).unwrap();
    let child = m.require(ids::SUBMODULE_CHILD).unwrap();

    assert!(parent.path.join(".gitmodules").is_file());
    let marker = child.path.join(".git");
    assert!(marker.is_file());
    assert!(std::fs::read_to_string(&marker)
        .unwrap()
        .contains("modules"));

    assert_ne!(common_dir(&m, &child.path), common_dir(&m, &parent.path));
    assert_ne!(child.expect.root_oids, parent.expect.root_oids);
    assert_eq!(
        child.expect.parent_fixture.as_deref(),
        Some(ids::SUBMODULE_PARENT)
    );
    assert_eq!(child.expect.submodule_path.as_deref(), Some("sub"));
}

#[test]
fn the_nested_submodule_is_checked_out_and_is_a_third_repository() {
    let m = build(&[ids::SUBMODULE_PARENT], "submodule-nested");
    let child = m.require(ids::SUBMODULE_CHILD).unwrap();
    let nested = m.require(ids::SUBMODULE_NESTED).unwrap();

    assert!(nested.path.starts_with(&child.path));
    assert!(nested.path.join(".git").is_file());
    assert_ne!(nested.expect.root_oids, child.expect.root_oids);
    assert_eq!(
        nested.expect.parent_fixture.as_deref(),
        Some(ids::SUBMODULE_CHILD)
    );
    assert_eq!(nested.expect.submodule_path.as_deref(), Some("inner"));
}

#[test]
fn asking_for_the_child_alone_still_builds_the_parent_that_contains_it() {
    assert_eq!(
        ids::dependencies(ids::SUBMODULE_CHILD),
        &[ids::SUBMODULE_PARENT]
    );
    assert_eq!(
        ids::dependencies(ids::SUBMODULE_NESTED),
        &[ids::SUBMODULE_PARENT]
    );
    assert_eq!(
        ids::dependencies(ids::LINKED_WORKTREE),
        &[ids::WORKTREE_PARENT]
    );
}

#[test]
fn the_submodule_sources_live_outside_both_volumes() {
    let m = build(&[ids::SUBMODULE_PARENT], "submodule-sources");
    for volume in &m.volumes {
        assert!(!m.root.join("sources").starts_with(&volume.path));
    }
}

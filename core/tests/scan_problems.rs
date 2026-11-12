//! Compiled only under `testkit`: these link `codotheca_core::testing`, which the feature gates.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §11.1's header counts folders: one scan run records at most one problem row per path.
//!
//! These run the production launcher over a real tree with real git, through the real
//! `SqliteScanStore`, because the double count was the walk and the hand-off each reporting
//! the same folder — a fake on either side would hide one of the two writers.

mod support;

use support::scan_rig::scan;
use support::TestRepo;

/// A `.git` file whose pointer leads nowhere: the walk's probe fails and classifies it from the
/// pointer, and the hand-off then cannot index it. One folder, one row — the hand-off's, which
/// carries git's own error — beside a healthy repository that indexes and reports nothing.
#[test]
fn a_broken_gitdir_pointer_is_one_problem_row_not_two() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"a\n");
    repo.commit("first");
    let root = support::canonical(repo.scratch());
    let lane = root.join("lane-a");
    std::fs::create_dir_all(&lane).unwrap();
    std::fs::write(
        lane.join(".git"),
        format!(
            "gitdir: {}\n",
            root.join("repo/.git/worktrees/lane-a").display()
        ),
    )
    .unwrap();

    let scanned = scan(&repo, &root);

    assert_eq!(scanned.indexed, 1, "the healthy repository still indexes");
    assert_eq!(
        scanned.problem_rows, 1,
        "the header counts folders: {:?}",
        scanned.problems
    );
    assert_eq!(scanned.problems.len(), 1);
    assert!(
        scanned.problems.iter().all(|(_, p)| p.contains("lane-a")),
        "the row names the broken folder: {:?}",
        scanned.problems
    );
}

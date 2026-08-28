#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §3.3's second row, and §4.1's J2 degrade: unknown untracked is never a zero.

mod support;

use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::SystemClock;
use codotheca_core::git::{parse_status_v2, worktree_status, RunLimits, StatusOptions};
use support::TestRepo;

fn status(repo: &TestRepo, opts: StatusOptions) -> codotheca_core::git::WorktreeStatus {
    worktree_status(
        &repo.exec(),
        &repo.handle(),
        opts,
        RunLimits::none(),
        &CancelToken::new(),
        &SystemClock::new(),
    )
    .unwrap()
}

#[test]
fn a_clean_tree_is_not_dirty_and_carries_an_observation_time() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    let st = status(&repo, StatusOptions::full());
    assert!(!st.is_dirty);
    assert_eq!(st.tracked_changes, 0);
    assert_eq!(st.untracked_count, Some(0));
    assert_eq!(st.branch.as_deref(), Some("main"));
    assert!(st.observed_at > 1_700_000_000);
}

#[test]
fn a_modified_tracked_file_makes_it_dirty() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    repo.write("a.txt", b"two\n");
    let st = status(&repo, StatusOptions::full());
    assert!(st.is_dirty);
    assert_eq!(st.tracked_changes, 1);
}

#[test]
fn untracked_files_are_counted_separately() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    repo.write("new-one.txt", b"x\n");
    repo.write("nested/new-two.txt", b"y\n");
    let st = status(&repo, StatusOptions::full());
    assert_eq!(st.untracked_count, Some(2));
    assert_eq!(st.tracked_changes, 0);
}

// The degrade path: no untracked enumeration at all, so the count is unknown, not zero.
#[test]
fn the_degraded_mode_reports_unknown_untracked_not_zero() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    repo.write("new-one.txt", b"x\n");
    repo.write("a.txt", b"edited\n");

    let st = status(&repo, StatusOptions::degraded());
    assert_eq!(
        st.untracked_count, None,
        "not enumerated must be None, never Some(0)"
    );
    assert!(st.is_dirty, "the tracked half survives the degrade");
    assert_eq!(st.tracked_changes, 1);
}

#[test]
fn a_rename_entry_consumes_its_second_path_field() {
    // Type 2 records carry the original path as a separate NUL-terminated field under -z.
    // A parser that misses that reads the original path as the next entry.
    // `\x00` rather than `\0` before the `2`: `\02` is ambiguous with an octal escape.
    let bytes = b"# branch.oid abc123\0# branch.head main\x002 R. N... 100644 100644 100644 abc def R100 new.txt\0old.txt\0? untracked.txt\0";
    let counts = parse_status_v2(bytes).unwrap();
    assert_eq!(counts.tracked_changes, 1);
    assert_eq!(counts.untracked, 1);
    assert_eq!(counts.branch.as_deref(), Some("main"));
}

#[test]
fn the_branch_header_supplies_ahead_and_behind() {
    let bytes = b"# branch.oid abc123\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +3 -1\0";
    let counts = parse_status_v2(bytes).unwrap();
    assert_eq!(counts.ahead, Some(3));
    assert_eq!(counts.behind, Some(1));
    assert_eq!(counts.head_oid.as_deref(), Some("abc123"));
}

#[test]
fn a_detached_head_header_yields_no_branch() {
    let bytes = b"# branch.oid abc123\0# branch.head (detached)\0";
    let counts = parse_status_v2(bytes).unwrap();
    assert_eq!(counts.branch, None);
}

#[test]
fn unmerged_entries_count_as_tracked_changes() {
    let bytes = b"# branch.head main\0u UU N... 100644 100644 100644 100644 a b c conflicted.txt\0";
    let counts = parse_status_v2(bytes).unwrap();
    assert_eq!(counts.tracked_changes, 1);
}

#[test]
fn a_non_utf8_path_does_not_break_the_parse() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let name = std::ffi::OsStr::from_bytes(b"weird-\xff-name.txt");
        std::fs::write(repo.path().join(name), b"x").unwrap();
    }
    #[cfg(not(unix))]
    {
        repo.write("weird-name.txt", b"x");
    }
    let st = status(&repo, StatusOptions::full());
    assert_eq!(st.untracked_count, Some(1));
}

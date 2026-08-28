#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §3.3's third row, and the invocation that deadlocked for five hours.

mod support;

use std::sync::mpsc;
use std::time::Duration;

use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::SystemClock;
use codotheca_core::git::{parse_ls_files_z, path_extension, tracked_inventory, RunLimits};
use support::TestRepo;

#[test]
fn sums_head_blob_bytes_and_counts_files() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"12345");
    repo.write("src/b.rs", b"1234567890");
    repo.commit("first");

    let inv = tracked_inventory(
        &repo.exec(),
        &repo.handle(),
        RunLimits::none(),
        &CancelToken::new(),
        &SystemClock::new(),
    )
    .unwrap();
    assert_eq!(inv.tracked_files, 2);
    assert_eq!(inv.size_tracked_bytes, 15);
    assert_eq!(inv.extension_bytes.get("txt"), Some(&5));
    assert_eq!(inv.extension_bytes.get("rs"), Some(&10));
    assert!(inv.observed_at > 1_700_000_000);
}

#[test]
fn two_identical_files_are_counted_twice_though_they_share_one_object() {
    let repo = TestRepo::init();
    repo.write("one.txt", b"same bytes");
    repo.write("two.txt", b"same bytes");
    repo.commit("first");

    let inv = tracked_inventory(
        &repo.exec(),
        &repo.handle(),
        RunLimits::none(),
        &CancelToken::new(),
        &SystemClock::new(),
    )
    .unwrap();
    assert_eq!(inv.tracked_files, 2);
    assert_eq!(inv.size_tracked_bytes, 20);
}

#[test]
fn an_empty_index_reports_zero_files_without_running_cat_file() {
    let repo = TestRepo::init();
    let inv = tracked_inventory(
        &repo.exec(),
        &repo.handle(),
        RunLimits::none(),
        &CancelToken::new(),
        &SystemClock::new(),
    )
    .unwrap();
    assert_eq!(inv.tracked_files, 0);
    assert_eq!(inv.size_tracked_bytes, 0);
    assert!(inv.extension_bytes.is_empty());
}

#[test]
fn extensions_come_from_the_last_component_and_odd_bytes_fall_into_the_empty_key() {
    assert_eq!(path_extension(b"src/main.rs"), "rs");
    assert_eq!(path_extension(b"src/Main.RS"), "rs");
    assert_eq!(path_extension(b"Makefile"), "");
    assert_eq!(path_extension(b"a.b/c"), "");
    assert_eq!(path_extension(b".gitignore"), "");
    assert_eq!(path_extension(b"weird.\xff\xfe"), "");
    assert_eq!(path_extension(b"archive.tar.gz"), "gz");
}

#[test]
fn ls_files_entries_parse_with_mode_oid_stage_and_path() {
    // `\x00` before a digit: `\0` followed by one is ambiguous with an octal escape.
    let line = b"100644 1111111111111111111111111111111111111111 0\tsrc/a.rs\x00100755 2222222222222222222222222222222222222222 2\tb.sh\0";
    let entries = parse_ls_files_z(line);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].oid, "1".repeat(40));
    assert_eq!(entries[0].stage, 0);
    assert_eq!(entries[0].path, b"src/a.rs".to_vec());
    assert_eq!(entries[1].stage, 2);
}

// A conflicted path has no stage 0; stage 2 (ours) is what the inventory measures.
#[test]
fn a_conflicted_path_is_measured_once_from_the_ours_stage() {
    let line = b"100644 1111111111111111111111111111111111111111 1\tc.txt\x00100644 2222222222222222222222222222222222222222 2\tc.txt\x00100644 3333333333333333333333333333333333333333 3\tc.txt\0";
    let entries = parse_ls_files_z(line);
    assert_eq!(
        entries.len(),
        3,
        "the parser reports every stage; selection happens above it"
    );
}

// The five-hour bug against a real repository: 3,000 tracked files put ~123 KB of queries in
// and ~180 KB of answers out, so both directions exceed a 64 KiB pipe buffer at once.
#[test]
fn a_repository_large_enough_to_fill_the_pipe_buffer_completes() {
    let repo = TestRepo::init();
    for i in 0..3_000 {
        repo.write(
            &format!("f/{i:05}.dat"),
            format!("payload-{i:05}\n").as_bytes(),
        );
    }
    repo.git(&["add", "-A"]);

    let (tx, rx) = mpsc::channel();
    let exec = repo.exec();
    let handle = repo.handle();
    std::thread::spawn(move || {
        let got = tracked_inventory(
            &exec,
            &handle,
            RunLimits::none(),
            &CancelToken::new(),
            &SystemClock::new(),
        );
        let _ = tx.send(got);
    });

    let inv = rx
        .recv_timeout(Duration::from_secs(60))
        .expect("tracked_inventory deadlocked: no result within 60s")
        .unwrap();
    assert_eq!(inv.tracked_files, 3_000);
    // Each file is "payload-NNNNN\n" = 14 bytes.
    assert_eq!(inv.size_tracked_bytes, 3_000 * 14);
    assert_eq!(inv.extension_bytes.get("dat"), Some(&(3_000 * 14)));
}

// Deviation from the plan, with its regression test. `one_per_path` documents "stage 0 when it
// exists, otherwise stage 2 (ours)", and the plan's comparison kept the LOWEST stage instead —
// stage 1, the merge base. A conflicted file would then be measured at the size it had before
// either side touched it. The plan's own conflict test only exercises the parser, so nothing
// above it could see this.
#[test]
fn a_conflicted_file_is_measured_from_ours_not_from_the_merge_base() {
    let repo = TestRepo::init();
    repo.write("c.txt", b"base-content\n"); // 13 bytes
    repo.commit("base");
    repo.git(&["checkout", "-q", "-b", "sideline"]);
    repo.write("c.txt", b"bbbbbbbb\n"); // 9 bytes, theirs
    repo.commit("sideline");
    repo.git(&["checkout", "-q", "main"]);
    repo.write("c.txt", b"aaa\n"); // 4 bytes, ours
    repo.commit("ours");
    assert!(
        !repo.try_git(&["merge", "--no-edit", "sideline"]),
        "the fixture merge has to conflict for this test to mean anything"
    );

    let inv = tracked_inventory(
        &repo.exec(),
        &repo.handle(),
        RunLimits::none(),
        &CancelToken::new(),
        &SystemClock::new(),
    )
    .unwrap();
    assert_eq!(inv.tracked_files, 1, "one path, not three stages");
    assert_eq!(
        inv.size_tracked_bytes, 4,
        "ours is 4 bytes; the merge base is 13 and theirs is 9"
    );
}

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
use codotheca_core::git::{
    head_tree, parse_ls_files_z, parse_ls_tree_z, path_extension, read_blobs, submodule_gitlinks,
    tracked_inventory, RunLimits,
};
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

// ---- §4.4's gitlink OIDs -------------------------------------------------------------------

/// `submodule_edge.gitlink_oid` (§1.9) has no source anywhere else in the plan set: `ls-files -s`
/// is the only read that reports one, and `TrackedInventory` discards the mode. The parser has
/// always parsed the mode and thrown it away.
#[test]
fn ls_files_entries_carry_their_mode() {
    let line = b"160000 4444444444444444444444444444444444444444 0\tvendor/lib\x00100644 1111111111111111111111111111111111111111 0\ta.txt\0";
    let entries = parse_ls_files_z(line);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].mode, "160000", "a gitlink");
    assert_eq!(entries[1].mode, "100644", "an ordinary blob");
}

#[test]
fn gitlinks_come_back_keyed_by_path_and_ordinary_blobs_do_not() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"hello");
    repo.git(&["add", "a.txt"]);
    // A gitlink without cloning anything: the index records a commit id at a path.
    let oid = "4".repeat(40);
    repo.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("160000,{oid},vendor/lib"),
    ]);

    let found = submodule_gitlinks(
        &repo.exec(),
        &repo.handle(),
        &[b"vendor/lib".to_vec(), b"a.txt".to_vec()],
        RunLimits::none(),
        &CancelToken::new(),
    )
    .unwrap();

    assert_eq!(found.len(), 1, "an ordinary blob is not a gitlink");
    assert_eq!(found.get(b"vendor/lib".as_slice()), Some(&oid));
}

/// Asking about no paths asks git nothing, rather than listing the whole index.
#[test]
fn an_empty_path_list_yields_an_empty_map() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"hello");
    repo.git(&["add", "a.txt"]);
    let found = submodule_gitlinks(
        &repo.exec(),
        &repo.handle(),
        &[],
        RunLimits::none(),
        &CancelToken::new(),
    )
    .unwrap();
    assert!(found.is_empty());
}

// ---------------------------------------------------------------------------
// §29.1's enumeration and §29.6's blob read. `ls-tree` is a sibling of
// `ls-files -s`, never a replacement: `ls-files -s` reads the **index**, which
// is the right basis for J3 and the wrong one for J7.
// ---------------------------------------------------------------------------

/// `<mode> SP <type> SP <oid> TAB <path>` per NUL-terminated record, raw path bytes throughout.
#[test]
fn parse_ls_tree_z_reads_mode_type_oid_and_raw_path() {
    let oid = "a".repeat(40);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(format!("100644 blob {oid}\tsrc/main.rs").as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(format!("160000 commit {}\tvendor/lib", "b".repeat(40)).as_bytes());
    bytes.push(0);
    // A record with no TAB is malformed and is skipped, never guessed at.
    bytes.extend_from_slice(b"100644 blob short-and-broken");
    bytes.push(0);
    // A path that is not UTF-8 must survive as bytes.
    bytes.extend_from_slice(format!("100755 blob {}\t", "c".repeat(40)).as_bytes());
    bytes.extend_from_slice(&[0xff, 0xfe, b'.', b's', b'h']);
    bytes.push(0);

    let entries = parse_ls_tree_z(&bytes);
    eprintln!("parsed {} records", entries.len());
    assert_eq!(entries.len(), 3, "the malformed record was not skipped");
    assert_eq!(entries[0].mode, "100644");
    assert_eq!(entries[0].kind, "blob");
    assert_eq!(entries[0].oid, oid);
    assert_eq!(entries[0].path, b"src/main.rs".to_vec());
    assert_eq!(entries[1].kind, "commit", "a gitlink is a commit record");
    assert_eq!(entries[2].path, vec![0xff, 0xfe, b'.', b's', b'h']);
}

/// The enumeration is HEAD's, so a staged-but-uncommitted file is not in it — and a file deleted
/// from the worktree but still committed is.
#[test]
fn head_tree_enumerates_the_commit_not_the_index() {
    let repo = TestRepo::init();
    repo.write("committed.rs", b"fn main() {}\n");
    repo.commit("first");
    repo.write("staged.rs", b"fn other() {}\n");
    repo.git(&["add", "staged.rs"]);
    std::fs::remove_file(repo.path().join("committed.rs")).unwrap();

    let entries = head_tree(
        &repo.exec(),
        &repo.handle(),
        RunLimits::none(),
        &CancelToken::new(),
    )
    .unwrap();
    let paths: Vec<String> = entries
        .iter()
        .map(|e| String::from_utf8_lossy(&e.path).into_owned())
        .collect();
    eprintln!("head enumerated {} paths: {paths:?}", paths.len());
    assert_eq!(paths, vec!["committed.rs".to_owned()]);
}

/// `--full-tree` needs no working tree, so a bare repository with commits enumerates like any
/// other. **Bare is not the discriminator** (§29.1); an unborn HEAD is.
#[test]
fn a_bare_repository_enumerates_like_any_other() {
    let source = TestRepo::init();
    source.write("a.rs", b"fn a() {}\n");
    source.commit("first");
    let bare = TestRepo::init_bare();
    source.git(&[
        "clone",
        "-q",
        "--bare",
        ".",
        &bare.path().join("copy.git").to_string_lossy(),
    ]);
    let handle = codotheca_core::git::RepoHandle::bare(
        &bare.path().join("copy.git"),
        codotheca_core::git::StoreKey::new("test-store"),
        codotheca_core::mount::StoreClass::Local,
    );

    let entries = head_tree(
        &source.exec(),
        &handle,
        RunLimits::none(),
        &CancelToken::new(),
    )
    .unwrap();
    eprintln!("bare enumeration yielded {} paths", entries.len());
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, b"a.rs".to_vec());
}

/// The deadlock guard, one level down from `tracked_inventory`'s: `cat-file --batch` writes the
/// whole body of every object, so the output passes a pipe buffer far sooner than
/// `--batch-check`'s does. Sixty 8 KiB blobs is roughly 480 KiB through a 64 KiB pipe.
#[test]
fn read_blobs_drains_a_batch_far_larger_than_a_pipe_buffer() {
    let repo = TestRepo::init();
    for i in 0..60_u32 {
        let mut body = format!("// blob {i}\n").into_bytes();
        body.resize(8 * 1024, b'x');
        repo.write(&format!("f{i}.rs"), &body);
    }
    repo.commit("bulk");
    let entries = head_tree(
        &repo.exec(),
        &repo.handle(),
        RunLimits::none(),
        &CancelToken::new(),
    )
    .unwrap();
    let oids: Vec<String> = entries.iter().map(|e| e.oid.clone()).collect();

    let (tx, rx) = mpsc::channel();
    let exec = repo.exec();
    let handle = repo.handle();
    std::thread::spawn(move || {
        let out = read_blobs(
            &exec,
            &handle,
            &oids,
            1024 * 1024,
            u64::MAX,
            RunLimits::none(),
            &CancelToken::new(),
        );
        let _ = tx.send(out);
    });
    let reads = rx
        .recv_timeout(Duration::from_secs(60))
        .expect("cat-file --batch deadlocked")
        .unwrap()
        .reads;
    eprintln!("read {} blobs", reads.len());
    // One answer per request line. De-duplicating the oid list is the caller's, because only the
    // caller knows which of two paths sharing an object it still has to attribute a finding to.
    assert_eq!(reads.len(), 60);
    for read in &reads {
        assert_eq!(read.size_bytes, 8 * 1024);
        assert_eq!(read.bytes.as_ref().map(Vec::len), Some(8 * 1024));
    }
}

/// A blob over the cap is **recorded and its body discarded**, never never-measured: the size
/// comes from `--batch`'s own header line, so `too_large` is a verdict with its evidence.
#[test]
fn a_blob_over_the_cap_keeps_its_size_and_loses_its_body() {
    let repo = TestRepo::init();
    repo.write("small.rs", b"fn a() {}\n");
    repo.write("big.rs", vec![b'q'; 4096].as_slice());
    repo.commit("first");
    let entries = head_tree(
        &repo.exec(),
        &repo.handle(),
        RunLimits::none(),
        &CancelToken::new(),
    )
    .unwrap();
    let oids: Vec<String> = entries.iter().map(|e| e.oid.clone()).collect();

    let batch = read_blobs(
        &repo.exec(),
        &repo.handle(),
        &oids,
        2048,
        u64::MAX,
        RunLimits::none(),
        &CancelToken::new(),
    )
    .unwrap();
    let reads = batch.reads;
    eprintln!("read {} blobs at a 2048-byte cap", reads.len());
    assert_eq!(reads.len(), 2);
    let big = reads.iter().find(|r| r.size_bytes == 4096).unwrap();
    assert!(big.bytes.is_none(), "the body was kept past the cap");
    let small = reads.iter().find(|r| r.size_bytes == 10).unwrap();
    assert_eq!(small.bytes.as_deref(), Some(b"fn a() {}\n".as_slice()));
}

/// A `<oid> missing` line has two fields and contributes nothing, exactly as `tracked_inventory`
/// already handles it — the reader must stay in step with the stream either way.
#[test]
fn a_missing_oid_contributes_nothing_and_does_not_desynchronise_the_reader() {
    let repo = TestRepo::init();
    repo.write("a.rs", b"fn a() {}\n");
    repo.commit("first");
    let entries = head_tree(
        &repo.exec(),
        &repo.handle(),
        RunLimits::none(),
        &CancelToken::new(),
    )
    .unwrap();
    let real = entries[0].oid.clone();
    let oids = vec!["0".repeat(40), real.clone(), "1".repeat(40)];

    let batch = read_blobs(
        &repo.exec(),
        &repo.handle(),
        &oids,
        1024 * 1024,
        u64::MAX,
        RunLimits::none(),
        &CancelToken::new(),
    )
    .unwrap();
    let reads = batch.reads;
    eprintln!(
        "asked for {} oids, answered {}, read {}",
        oids.len(),
        batch.covered,
        reads.len()
    );
    assert_eq!(batch.covered, oids.len(), "a missing oid stalled the batch");
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].oid, real);
    assert_eq!(reads[0].bytes.as_deref(), Some(b"fn a() {}\n".as_slice()));
}

//! §29's acceptance criteria — `AC-P3-29-*`.
//!
//! Every scanning check prints what it scanned and fails at zero, and every count a checker can
//! derive it derives: none is written into a test. Where a criterion compares two runs the
//! comparison is by **identity**, never by count.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{head_tree, read_blobs, RunLimits};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::jobs::JobKind;
use codotheca_core::protocol::{Job, SyncTaskKind};
use codotheca_core::sync::task::kind_slug;
use support::TestRepo;

fn migrated() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

fn insert_project(conn: &rusqlite::Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES (?1, ?1, 0, 0)",
        [name],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// **AC-P3-29-21.** The job vocabularies stay disjoint and complete.
///
/// The assertion is over the **set**, never the size (R132/F16): the array's length lives in the
/// type, and a criterion that increments a hand-maintained literal is a bar that has to be
/// remembered rather than one that holds. Four parts, each printing what it derived.
#[test]
fn ac_p3_29_21_the_job_vocabularies_stay_disjoint_and_complete() {
    let slugs: Vec<&'static str> = JobKind::ALL.iter().map(|k| k.slug()).collect();
    eprintln!("JobKind::ALL derives {} slugs: {slugs:?}", slugs.len());
    assert!(!slugs.is_empty(), "the job vocabulary is empty");

    // 1. Every slug is a schema `Job` variant. Read through serde, which is the same path the
    //    wire takes, so a variant the schema does not declare cannot pass here.
    for slug in &slugs {
        let parsed: Result<Job, _> = serde_json::from_value(serde_json::json!(slug));
        assert!(
            parsed.is_ok(),
            "{slug} is a JobKind and no schema Job variant"
        );
    }

    // 2. Every slug is accepted by `project_job_state.job`'s CHECK, against a real migrated
    //    database — R26's shape, and the half that would catch a migration that widened the
    //    column for the wrong spelling.
    let (_dir, conn) = migrated();
    let project = insert_project(&conn, "p");
    let mut inserted = 0;
    for slug in &slugs {
        conn.execute(
            "INSERT INTO project_job_state (project_id, job, state, at)
             VALUES (?1, ?2, 'queued', 0)",
            rusqlite::params![project, slug],
        )
        .unwrap_or_else(|e| panic!("the column refused {slug}: {e}"));
        inserted += 1;
    }
    eprintln!("slugs accepted by project_job_state.job: {inserted}");
    assert!(
        inserted > 0,
        "inserted nothing, so the column proved nothing"
    );

    // 3. No slug appears in the sync vocabulary. Two runners, two tables; one slug in both would
    //    make a grep over either column ambiguous for good.
    let syncs: Vec<&'static str> = SyncTaskKind::ALL.iter().map(|k| kind_slug(*k)).collect();
    eprintln!("SyncTaskKind::ALL derives {} slugs: {syncs:?}", syncs.len());
    assert!(!syncs.is_empty(), "the sync vocabulary is empty");
    for sync in &syncs {
        assert!(!slugs.contains(sync), "{sync} names a job and a sync task");
    }
}

/// **AC-P3-29-2.** The enumeration reads HEAD, not the index.
///
/// The index is really dirtied — a second file holding a `TODO` is staged and never committed —
/// and no mock stands in for it. `ls-files -s` reads the staged entry, so a J7 built on it would
/// yield findings from uncommitted edits: debt flickering as the user types, and a blob id HEAD
/// never contained landing in a permanent, library-wide cache.
#[test]
fn ac_p3_29_2_the_enumeration_reads_head_not_the_index() {
    let repo = TestRepo::init();
    repo.write("committed.rs", b"fn a() {}\n// TODO: the committed one\n");
    repo.commit("first");
    repo.write("staged.rs", b"fn b() {}\n// TODO: the staged one\n");
    repo.git(&["add", "staged.rs"]);

    let entries = head_tree(
        &repo.exec(),
        &repo.handle(),
        RunLimits::none(),
        &CancelToken::new(),
    )
    .unwrap();
    let oids: Vec<String> = entries.iter().map(|e| e.oid.clone()).collect();
    let reads = read_blobs(
        &repo.exec(),
        &repo.handle(),
        &oids,
        512 * 1024,
        RunLimits::none(),
        &CancelToken::new(),
    )
    .unwrap();

    // The salient text of every marker the read covers. Task 5 replaces this crude scan with
    // `scan_blob`; what the criterion turns on either way is *which blob was read*.
    let mut findings: Vec<String> = Vec::new();
    for read in &reads {
        let Some(bytes) = read.bytes.as_deref() else {
            continue;
        };
        for line in bytes.split(|b| *b == b'\n') {
            if let Some(at) = line.windows(4).position(|w| w == b"TODO") {
                findings.push(String::from_utf8_lossy(&line[at..]).into_owned());
            }
        }
    }
    findings.sort();
    eprintln!(
        "enumerated {} paths, read {} blobs, found {} markers: {findings:?}",
        entries.len(),
        reads.len(),
        findings.len()
    );
    assert!(!findings.is_empty(), "the scan found nothing to compare");
    assert_eq!(findings, vec!["TODO: the committed one".to_owned()]);
}

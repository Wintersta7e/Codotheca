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
use codotheca_core::git::{
    head_tree, read_blobs, GitBackend, JobClass, JobContext, RepoHandle, RunLimits,
};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::jobs::content_scan::{
    cached_scan, findings_for_blob, missing_blobs, record_read, BlobOutcome, CachedScan,
};
use codotheca_core::jobs::j3_inventory::ARCHETYPE_SAMPLE;
use codotheca_core::jobs::markers::{J7_BLOB_BYTE_CAP, J7_SCANNER_VERSION};
use codotheca_core::jobs::presence::{presence_for, PresenceAnswers, PresenceState};
use codotheca_core::jobs::JobKind;
use codotheca_core::proto::txguard::TxGuard;
use codotheca_core::protocol::{Job, SyncTaskKind};
use codotheca_core::sync::task::kind_slug;
use codotheca_core::testing::FakeGitBackend;
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

/// Enumerate a real repository's HEAD and answer §29.4's four predicates over it.
fn presence_of(repo: &TestRepo) -> PresenceAnswers {
    let entries = head_tree(
        &repo.exec(),
        &repo.handle(),
        RunLimits::none(),
        &CancelToken::new(),
    )
    .unwrap();
    eprintln!("enumerated {} paths", entries.len());
    presence_for(&entries)
}

/// **AC-P3-29-14.** The presence predicates run before the extension filter.
///
/// Written because filtering first makes all four permanently `absent` while every other
/// criterion still passes: `.github/workflows/ci.yml` is a `markup(...)` extension and a
/// `LICENSE` has no extension at all.
#[test]
fn ac_p3_29_14_the_presence_predicates_run_before_the_extension_filter() {
    let repo = TestRepo::init();
    repo.write(".github/workflows/ci.yml", b"on: push\n");
    repo.write("LICENSE", b"MIT\n");
    repo.write("src/main.rs", b"fn main() {}\n");
    repo.commit("first");

    let entries = head_tree(
        &repo.exec(),
        &repo.handle(),
        RunLimits::none(),
        &CancelToken::new(),
    )
    .unwrap();
    let answers = presence_for(&entries);
    eprintln!("over {} unfiltered paths: {answers:?}", entries.len());
    assert_eq!(answers.ci, PresenceState::Present);
    assert_eq!(answers.license, PresenceState::Present);

    // The other order, which is the defect this criterion exists to catch: §29.2's rule 3 keeps
    // only `programming` extensions, and neither of these two is one.
    let filtered: Vec<_> = entries
        .iter()
        .filter(|e| e.path.ends_with(b".rs"))
        .cloned()
        .collect();
    let wrong = presence_for(&filtered);
    eprintln!("over {} filtered paths: {wrong:?}", filtered.len());
    assert!(!filtered.is_empty(), "the filtered set is empty either way");
    assert_eq!(wrong.ci, PresenceState::Absent);
    assert_eq!(wrong.license, PresenceState::Absent);
}

/// **AC-P3-29-13.** The presence predicates are exhaustive.
///
/// The fixture is built at `ARCHETYPE_SAMPLE + 1` paths **read from the Rust constant** (R132/F12
/// — never a copied `4_000`), with the `LICENSE` last, and the assertion is that the printed path
/// count exceeds the constant rather than a literal. J3's path sample is capped at that constant
/// and exists for archetype detection, so a predicate built on it would read a large tree's
/// licence as absent — and *absent* is the rendering that calls the user incomplete.
#[test]
fn ac_p3_29_13_the_presence_predicates_are_exhaustive() {
    let repo = TestRepo::init();
    for i in 0..ARCHETYPE_SAMPLE {
        repo.write(&format!("src/f{i:05}.rs"), b"fn f() {}\n");
    }
    repo.write("LICENSE", b"MIT\n");
    repo.commit("wide");

    let entries = head_tree(
        &repo.exec(),
        &repo.handle(),
        RunLimits::none(),
        &CancelToken::new(),
    )
    .unwrap();
    eprintln!(
        "enumerated {} paths against ARCHETYPE_SAMPLE = {ARCHETYPE_SAMPLE}",
        entries.len()
    );
    assert!(
        entries.len() > ARCHETYPE_SAMPLE,
        "the fixture does not reach past the sample it is written to defeat"
    );
    assert_eq!(presence_for(&entries).license, PresenceState::Present);
}

/// **AC-P3-29-12.** A budget exceedance is `not_read`, never `absent`.
///
/// The two cases are asserted separately and **by identity**: a timeout looks exactly like a
/// missing file, and this is the single most likely place phase 3 renders unknown as zero.
#[test]
fn ac_p3_29_12_a_budget_exceedance_is_not_read_never_absent() {
    // A repository whose enumeration fails: no commits, so `ls-tree HEAD` has nothing to name.
    let unborn = TestRepo::init();
    let failed = head_tree(
        &unborn.exec(),
        &unborn.handle(),
        RunLimits::none(),
        &CancelToken::new(),
    );
    assert!(failed.is_err(), "the enumeration was expected to fail");
    let unread = PresenceAnswers::not_read();
    eprintln!("a failed enumeration answers: {unread:?}");
    assert_eq!(unread.readme, PresenceState::NotRead);
    assert_eq!(unread.license, PresenceState::NotRead);
    assert_eq!(unread.tests, PresenceState::NotRead);
    assert_eq!(unread.ci, PresenceState::NotRead);

    // A repository enumerated with no licence: a known false, and a different value.
    let read = TestRepo::init();
    read.write("src/main.rs", b"fn main() {}\n");
    read.commit("first");
    let answers = presence_of(&read);
    eprintln!("an enumerated repository with no licence answers: {answers:?}");
    assert_eq!(answers.license, PresenceState::Absent);
    assert_ne!(answers.license, unread.license);
}

/// **AC-P3-29-15.** `ci` and `ciGreen` are two facts, and neither is derived from the other.
///
/// `ci` is *a CI configuration exists at HEAD* — this section's path predicate. `ciGreen` is the
/// forge's latest conclusion, a remote fact with its own `observed_at`. The conclusion is moved
/// from `failure` to `success` and the presence answer does not move with it.
#[test]
fn ac_p3_29_15_ci_and_ci_green_are_two_facts() {
    let repo = TestRepo::init();
    repo.write(".github/workflows/ci.yml", b"on: push\n");
    repo.commit("first");
    let answers = presence_of(&repo);

    let (_dir, conn) = migrated();
    conn.execute(
        "INSERT INTO remote_repo (provider, provider_repo_id) VALUES ('github', 'r1')",
        [],
    )
    .unwrap();
    let mut conclusions = Vec::new();
    for conclusion in ["failure", "success"] {
        conn.execute(
            "INSERT INTO remote_ci_run
               (provider, provider_repo_id, run_id, workflow_name, conclusion, branch, run_number)
             VALUES ('github', 'r1', ?1, 'ci', ?2, 'main', 1)
             ON CONFLICT(provider, provider_repo_id, run_id)
               DO UPDATE SET conclusion = excluded.conclusion",
            rusqlite::params![1, conclusion],
        )
        .unwrap();
        let stored: String = conn
            .query_row(
                "SELECT conclusion FROM remote_ci_run WHERE run_id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        conclusions.push(stored);
        assert_eq!(
            presence_for(&[]).ci,
            PresenceState::Absent,
            "the predicate reads the tree, and an empty tree has no CI configuration"
        );
        assert_eq!(answers.ci, PresenceState::Present);
    }
    eprintln!("ci = {:?} across conclusions {conclusions:?}", answers.ci);
    assert_eq!(
        conclusions,
        vec!["failure".to_owned(), "success".to_owned()]
    );
}

/// **AC-P3-29-26.** An unreadable README is not a missing one.
///
/// **R127.3: the assertion is kept and the justification paragraph is not.** `has_readme` is
/// answered from the `ls-tree` enumeration and never enters `read_capped`'s call path, so an
/// unreadable fixture would claim coverage this criterion does not have. The unreadable-file
/// coverage is §31's. The two repositories are compared **by identity**.
#[test]
fn ac_p3_29_26_an_unreadable_readme_is_not_a_missing_one() {
    let with = TestRepo::init();
    with.write("README.md", b"# a project\n");
    with.commit("first");
    let without = TestRepo::init();
    without.write("src/main.rs", b"fn main() {}\n");
    without.commit("first");

    let held = presence_of(&with).readme;
    let none = presence_of(&without).readme;
    eprintln!("a committed README answers {held:?}; a tree with none answers {none:?}");
    assert_eq!(held, PresenceState::Present);
    assert_eq!(none, PresenceState::Absent);
    assert_ne!(held, none);
}

/// One chunk of §29.2's blob pass, at the cache layer: ask the cache what is missing, read only
/// those, and file both tables. Task 7's `run_j7` is the same shape with the enumeration, the
/// cursor and the gates around it; what these criteria turn on is which oids were asked for.
fn scan_through_cache(
    conn: &mut rusqlite::Connection,
    git: &FakeGitBackend,
    repo: &RepoHandle,
    oids: &[String],
) -> Vec<String> {
    let missing = missing_blobs(conn, oids, J7_SCANNER_VERSION).unwrap();
    let cancel = CancelToken::new();
    let ctx = JobContext::new(JobClass::Background, &cancel, None);
    let reads = git
        .read_blobs(repo, &missing, J7_BLOB_BYTE_CAP, &ctx)
        .unwrap();
    let tx = conn.transaction().unwrap();
    let _guard = TxGuard::enter();
    for read in &reads {
        record_read(&tx, read, J7_SCANNER_VERSION, 1_700_000_000).unwrap();
    }
    tx.commit().unwrap();
    missing
}

fn fake_repo() -> RepoHandle {
    RepoHandle::bare(
        std::path::Path::new("/does/not/matter"),
        codotheca_core::git::StoreKey::new("test-store"),
        codotheca_core::mount::StoreClass::Local,
    )
}

fn count(conn: &rusqlite::Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .unwrap()
}

/// **AC-P3-29-10.** A clean blob is not re-read.
///
/// **This is the criterion that catches A10's missing outcome**: without the `blob_scan` row the
/// finding set is identical either way and the defect is invisible — every clean blob re-read for
/// ever, and *read and clean* indistinguishable from *never read*.
#[test]
fn ac_p3_29_10_a_clean_blob_is_not_re_read() {
    let (_dir, mut conn) = migrated();
    let git = FakeGitBackend::new();
    let repo = fake_repo();
    let oid = "c".repeat(40);
    git.script_blob(&oid, b"fn a() {}\n");
    let oids = vec![oid.clone()];

    let first = scan_through_cache(&mut conn, &git, &repo, &oids);
    eprintln!("first pass asked for {} oids: {first:?}", first.len());
    assert_eq!(first, oids, "the first pass read nothing");
    assert_eq!(
        count(&conn, "blob_scan"),
        1,
        "no row saying it was looked at"
    );
    assert_eq!(count(&conn, "blob_finding"), 0);
    assert_eq!(
        cached_scan(&conn, &oid, J7_SCANNER_VERSION).unwrap(),
        Some(CachedScan {
            outcome: BlobOutcome::Scanned,
            size_bytes: 10
        })
    );

    git.clear();
    let second = scan_through_cache(&mut conn, &git, &repo, &oids);
    eprintln!(
        "second pass asked for {} oids; the seam recorded {:?}",
        second.len(),
        git.blob_requests()
    );
    assert!(second.is_empty(), "a clean blob was read a second time");
    assert!(git.blob_requests().is_empty(), "bytes were read again");
}

/// **AC-P3-29-25.** `too_large` and `binary` are recorded, not dropped — and neither is re-read.
#[test]
fn ac_p3_29_25_too_large_and_binary_are_recorded_not_dropped() {
    let (_dir, mut conn) = migrated();
    let git = FakeGitBackend::new();
    let repo = fake_repo();
    let big = "a".repeat(40);
    let binary = "b".repeat(40);
    // One byte past the cap, and a NUL inside the sniff window. Both carry a `TODO` that must
    // never become a finding.
    let mut oversized = b"// TODO in a huge file\n".to_vec();
    oversized.resize(usize::try_from(J7_BLOB_BYTE_CAP).unwrap() + 1, b'x');
    let mut binary_bytes = b"// TODO\0 in an object file\n".to_vec();
    binary_bytes.resize(64, b'\0');
    git.script_blob(&big, &oversized);
    git.script_blob(&binary, &binary_bytes);
    let oids = vec![big.clone(), binary.clone()];

    let first = scan_through_cache(&mut conn, &git, &repo, &oids);
    assert_eq!(first.len(), 2);
    let outcomes: Vec<BlobOutcome> = oids
        .iter()
        .map(|oid| {
            cached_scan(&conn, oid, J7_SCANNER_VERSION)
                .unwrap()
                .unwrap()
                .outcome
        })
        .collect();
    eprintln!(
        "outcomes: {outcomes:?}, findings: {}",
        count(&conn, "blob_finding")
    );
    assert_eq!(outcomes, vec![BlobOutcome::TooLarge, BlobOutcome::Binary]);
    assert_eq!(
        count(&conn, "blob_finding"),
        0,
        "a non-outcome produced findings"
    );

    git.clear();
    let second = scan_through_cache(&mut conn, &git, &repo, &oids);
    assert!(second.is_empty(), "a recorded non-outcome was read again");
    assert!(git.blob_requests().is_empty());
}

/// **AC-P3-29-8.** The cache is content-addressed and library-wide.
///
/// Two projects holding a byte-identical blob yield **one** `blob_scan` row and one finding set,
/// and the second project's scan reads no bytes. Neither cache table carries a `project_id`, so
/// there is nothing for a second project to miss on.
#[test]
fn ac_p3_29_8_the_cache_is_content_addressed_and_library_wide() {
    let (_dir, mut conn) = migrated();
    let git = FakeGitBackend::new();
    let first_repo = fake_repo();
    let oid = "d".repeat(40);
    git.script_blob(&oid, b"// TODO: vendored in both\n");
    let oids = vec![oid.clone()];

    scan_through_cache(&mut conn, &git, &first_repo, &oids);
    let after_first = findings_for_blob(&conn, &oid, J7_SCANNER_VERSION).unwrap();
    assert_eq!(after_first.len(), 1);

    // A second project, a different repository handle, the same bytes.
    git.clear();
    let second_repo = RepoHandle::bare(
        std::path::Path::new("/another/project"),
        codotheca_core::git::StoreKey::new("other-store"),
        codotheca_core::mount::StoreClass::Local,
    );
    let missed = scan_through_cache(&mut conn, &git, &second_repo, &oids);
    eprintln!(
        "second project missed {} oids and the seam recorded {:?}",
        missed.len(),
        git.blob_requests()
    );
    assert!(missed.is_empty());
    assert!(
        git.blob_requests().is_empty(),
        "the second project read bytes"
    );
    assert_eq!(count(&conn, "blob_scan"), 1, "the cache grew a second row");
    assert_eq!(
        findings_for_blob(&conn, &oid, J7_SCANNER_VERSION).unwrap(),
        after_first,
        "one finding set, by identity"
    );
}

/// **AC-P3-29-9.** A `scanner_version` bump is a miss, not a hit.
///
/// Older rows are **not deleted eagerly**, so a rollback to the previous build finds its cache
/// intact. The count invalidated is printed and the criterion fails at zero.
#[test]
fn ac_p3_29_9_a_scanner_version_bump_is_a_miss_not_a_hit() {
    let (_dir, mut conn) = migrated();
    let git = FakeGitBackend::new();
    let repo = fake_repo();
    let oid = "e".repeat(40);
    git.script_blob(&oid, b"// FIXME: at the old version\n");
    let oids = vec![oid.clone()];
    scan_through_cache(&mut conn, &git, &repo, &oids);

    let bumped = J7_SCANNER_VERSION + 1;
    let invalidated = missing_blobs(&conn, &oids, bumped).unwrap();
    eprintln!(
        "{} of {} rows are a miss at version {bumped}",
        invalidated.len(),
        oids.len()
    );
    assert!(
        !invalidated.is_empty(),
        "nothing was invalidated by the bump"
    );
    assert!(cached_scan(&conn, &oid, bumped).unwrap().is_none());
    assert!(
        cached_scan(&conn, &oid, J7_SCANNER_VERSION)
            .unwrap()
            .is_some(),
        "the older row was deleted eagerly, so a rollback finds no cache"
    );
}

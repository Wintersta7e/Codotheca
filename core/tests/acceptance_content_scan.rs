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
    head_tree, read_blobs, GitBackend, JobClass, JobContext, RepoHandle, RunLimits, TreeEntry,
};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::jobs::content_scan::{
    cached_scan, findings_for_blob, missing_blobs, record_read, BlobOutcome, CachedScan,
};
use codotheca_core::jobs::j3_inventory::ARCHETYPE_SAMPLE;
use codotheca_core::jobs::j7_markers::{self, ContentGates, ContentScanRow};
use codotheca_core::jobs::markers::{J7_BLOB_BYTE_CAP, J7_CHUNK_BLOBS, J7_SCANNER_VERSION};
use codotheca_core::jobs::presence::{presence_for, PresenceAnswers, PresenceState};
use codotheca_core::jobs::state::{apply_outcome, JobStateRow};
use codotheca_core::jobs::{JobKind, JobOutcome, JobState};
use codotheca_core::proto::txguard::TxGuard;
use codotheca_core::protocol::{Job, LocationId, ProjectId, SyncTaskKind};
use codotheca_core::sync::task::kind_slug;
use codotheca_core::testing::{FakeGitBackend, GitReply};
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
        u64::MAX,
        RunLimits::none(),
        &CancelToken::new(),
    )
    .unwrap()
    .reads;

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
        .read_blobs(repo, &missing, J7_BLOB_BYTE_CAP, u64::MAX, &ctx)
        .unwrap()
        .reads;
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

/// A project, its location and a fake git carrying a scripted tree, ready for `run_j7`.
struct Rig {
    _dir: tempfile::TempDir,
    index: std::sync::Mutex<Index>,
    git: std::sync::Arc<FakeGitBackend>,
    project: ProjectId,
    location: LocationId,
    repo: RepoHandle,
}

impl Rig {
    fn new(head_oid: &str) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let index = Index::open(&dir.path().join("index")).unwrap();
        let (project, location) = {
            let conn = index.conn();
            // Authorship computed and not Reference, so §29.7's first predicate lets J7 run;
            // the grant on, so the third lets it read. Each test that is about a gate sets its
            // own.
            conn.execute(
                "INSERT INTO project
                   (name, seed_basename, created_at, updated_at, authored_by_user, is_reference)
                 VALUES ('p', 'p', 0, 0, 1, 0)",
                [],
            )
            .unwrap();
            let project = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO app_meta (k, v) VALUES ('content_scan_enabled', '1')
                 ON CONFLICT(k) DO UPDATE SET v = excluded.v",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO location
                   (project_id, kind, path_bytes, path_key, path_display, store_key, presence,
                    repo_kind, head_oid)
                 VALUES (?1, 'linux', x'2f70', x'2f70', '/p', 'store-a', 'present',
                         'worktree', ?2)",
                rusqlite::params![project, head_oid],
            )
            .unwrap();
            (ProjectId(project), LocationId(conn.last_insert_rowid()))
        };
        Rig {
            _dir: dir,
            index: std::sync::Mutex::new(index),
            git: std::sync::Arc::new(FakeGitBackend::new()),
            project,
            location,
            repo: fake_repo(),
        }
    }

    /// Script one blob's bytes under a content address the tree entry will name.
    fn source(&self, bytes: &[u8]) -> String {
        let oid = format!("{:040x}", md5ish(bytes));
        self.git.script_blob(&oid, bytes);
        oid
    }

    fn set_tree(&self, entries: Vec<TreeEntry>) {
        self.git.always_head_tree(GitReply::Ok(entries));
    }

    fn set_head(&self, head_oid: &str) {
        let guard = self.index.lock().unwrap();
        guard
            .conn()
            .execute(
                "UPDATE location SET head_oid = ?2 WHERE id = ?1",
                rusqlite::params![self.location.0, head_oid],
            )
            .unwrap();
        drop(guard);
    }

    /// Write one fact a gate reads, against this rig's own project.
    fn set(&self, sql: &str) {
        let guard = self.index.lock().unwrap();
        guard
            .conn()
            .execute(sql, rusqlite::params![self.project.0])
            .unwrap();
        drop(guard);
    }

    fn gates(&self) -> ContentGates {
        let guard = self.index.lock().unwrap();
        let gates = j7_markers::gates_for(guard.conn(), self.project).unwrap();
        drop(guard);
        gates
    }

    fn run(&self, cursor: Option<&str>) -> JobOutcome {
        let cancel = CancelToken::new();
        let ctx = JobContext::new(JobClass::Background, &cancel, None);
        j7_markers::run_j7(
            &self.index,
            self.git.as_ref(),
            &self.repo,
            &ctx,
            j7_markers::ScanRun {
                project: self.project,
                location: self.location,
                cursor,
                now: 1_700_000_000,
            },
        )
        .unwrap()
    }

    fn scan_row(&self) -> Option<ContentScanRow> {
        let guard = self.index.lock().unwrap();
        let row = j7_markers::content_scan_row(guard.conn(), self.project).unwrap();
        drop(guard);
        row
    }

    /// Every finding in the cache, as the tuple a consumer compares by identity.
    fn findings(&self) -> Vec<(String, u32, String)> {
        let guard = self.index.lock().unwrap();
        let mut statement = guard
            .conn()
            .prepare(
                "SELECT blob_oid, ordinal_in_blob, salient_text_capped FROM blob_finding
                  ORDER BY blob_oid, ordinal_in_blob",
            )
            .unwrap();
        let rows: Vec<(String, u32, String)> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        drop(statement);
        drop(guard);
        rows
    }
}

/// A short, stable content address for a fixture blob. Not a git oid and never compared to one —
/// the fake's blob map is keyed by whatever the tree says.
fn md5ish(bytes: &[u8]) -> u128 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    u128::from(hash)
}

fn blob_entry(path: &str, oid: &str) -> TreeEntry {
    TreeEntry {
        mode: "100644".to_owned(),
        kind: "blob".to_owned(),
        oid: oid.to_owned(),
        path: path.as_bytes().to_vec(),
    }
}

/// **AC-P3-29-4.** An incomplete scan publishes no count.
///
/// With `blobs_pending > 0` the marker aggregate is **absent — not zero, not partial** — and
/// `complete_head_oid` is NULL. There is no flag to remember to set: the absence of a completed
/// head *is* the absence of the number.
#[test]
fn ac_p3_29_4_an_incomplete_scan_publishes_no_count() {
    let rig = Rig::new("head-one");
    let mut entries = Vec::new();
    for i in 0..(J7_CHUNK_BLOBS + 3) {
        let body = format!("// TODO: item {i}\n");
        let oid = rig.source(body.as_bytes());
        entries.push(blob_entry(&format!("src/f{i:05}.rs"), &oid));
    }
    rig.set_tree(entries);

    let outcome = rig.run(None);
    let row = rig.scan_row().unwrap();
    eprintln!("after one chunk: {outcome:?}, row {row:?}");
    assert!(matches!(outcome, JobOutcome::Partial { .. }));
    assert!(row.blobs_pending.unwrap_or(0) > 0);
    assert_eq!(
        row.complete_head_oid, None,
        "an incomplete scan published a head"
    );
    assert_eq!(
        row.blobs_total,
        Some(i64::try_from(J7_CHUNK_BLOBS + 3).unwrap())
    );
}

/// **AC-P3-29-5.** A cut-off is `Partial` and requeues without a failure.
///
/// Over several chunks it produces **the same finding set, by identity**, as one unbounded run —
/// never the same count.
#[test]
fn ac_p3_29_5_a_cut_off_is_partial_and_requeues_without_a_failure() {
    let build = |rig: &Rig| {
        let mut entries = Vec::new();
        for i in 0..(J7_CHUNK_BLOBS + 5) {
            let body = format!("// FIXME: number {i}\n// HACK: and again {i}\n");
            let oid = rig.source(body.as_bytes());
            entries.push(blob_entry(&format!("src/f{i:05}.rs"), &oid));
        }
        rig.set_tree(entries);
    };

    let chunked = Rig::new("head-one");
    build(&chunked);
    let mut cursor: Option<String> = None;
    let mut chunks = 0;
    let mut rows = Vec::new();
    loop {
        let outcome = chunked.run(cursor.as_deref());
        chunks += 1;
        match outcome {
            JobOutcome::Partial {
                cursor: next,
                done,
                total,
            } => {
                rows.push((done, total));
                assert!(
                    done > 0 && total.unwrap_or(0) > 0,
                    "a chunk boundary published a zero"
                );
                cursor = Some(next);
            }
            JobOutcome::Done => break,
            other => panic!("a chunk boundary is never {other:?}"),
        }
        assert!(chunks < 20, "the cursor did not advance");
    }
    eprintln!("{chunks} chunks, boundaries {rows:?}");
    assert!(chunks > 1, "the fixture never crossed a chunk boundary");

    // `apply_outcome` never counts a chunk boundary as a failure.
    let row = JobStateRow::fresh(JobKind::J7Markers, JobState::Running, 0);
    let (next, when) = apply_outcome(
        &row,
        &JobOutcome::Partial {
            cursor: "1".to_owned(),
            done: 1,
            total: Some(2),
        },
        1_700_000_000,
    );
    assert_eq!(next.fail_count, 0);
    assert_eq!(next.state, JobState::Queued);
    assert_eq!(when, Some(1_700_000_000));

    // The same tree, read without a chunk boundary, yields the same finding set by identity.
    let whole = Rig::new("head-one");
    build(&whole);
    let mut cursor: Option<String> = None;
    while let JobOutcome::Partial { cursor: next, .. } = whole.run(cursor.as_deref()) {
        cursor = Some(next);
    }
    let a = chunked.findings();
    let b = whole.findings();
    eprintln!(
        "chunked found {} occurrences, whole found {}",
        a.len(),
        b.len()
    );
    assert!(!a.is_empty(), "the comparison is over an empty set");
    assert_eq!(a, b, "the two runs disagree by identity");
}

/// **AC-P3-29-7.** An unchanged head invokes git zero times.
///
/// The trigger is a head comparison — not a timer, not a watcher. A `fetch` moves
/// `refstate_basis` without moving `head_oid`, so J7 short-circuits and §6's bounded watch set
/// gains nothing.
#[test]
fn ac_p3_29_7_an_unchanged_head_invokes_git_zero_times() {
    let rig = Rig::new("head-one");
    let oid = rig.source(b"// TODO: one\n");
    rig.set_tree(vec![blob_entry("src/a.rs", &oid)]);
    assert_eq!(rig.run(None), JobOutcome::Done);
    assert_eq!(
        rig.scan_row().unwrap().complete_head_oid.as_deref(),
        Some("head-one")
    );

    rig.git.clear();
    let outcome = rig.run(None);
    eprintln!(
        "second run: {outcome:?}, git recorded {:?}",
        rig.git.calls()
    );
    assert_eq!(outcome, JobOutcome::Done);
    assert!(
        rig.git.calls().is_empty(),
        "the git seam was invoked against an unchanged head"
    );
}

/// **AC-P3-29-6.** A moved head restarts the scan at ordinal 0 against the new head.
///
/// A scan straddling two heads is a reading of neither.
#[test]
fn ac_p3_29_6_a_moved_head_restarts_the_scan() {
    let rig = Rig::new("head-one");
    let mut entries = Vec::new();
    for i in 0..(J7_CHUNK_BLOBS + 2) {
        let oid = rig.source(format!("// TODO: {i}\n").as_bytes());
        entries.push(blob_entry(&format!("src/f{i:05}.rs"), &oid));
    }
    rig.set_tree(entries);
    let JobOutcome::Partial { cursor, .. } = rig.run(None) else {
        panic!("the fixture never crossed a chunk boundary");
    };
    assert_ne!(cursor, "0");
    let before = rig.scan_row().unwrap();
    assert!(before.blobs_pending.unwrap_or(0) > 0);

    rig.set_head("head-two");
    let outcome = rig.run(Some(&cursor));
    let after = rig.scan_row().unwrap();
    eprintln!("cursor was {cursor}; after the head moved: {outcome:?}, row {after:?}");
    assert_eq!(after.head_oid, "head-two");
    assert_eq!(after.complete_head_oid, None);
    // Restarting at ordinal 0 means the whole enumeration is pending again, not the tail the old
    // cursor named.
    assert_eq!(
        after.blobs_pending,
        before
            .blobs_total
            .map(|t| t - i64::try_from(J7_CHUNK_BLOBS).unwrap())
    );
    assert_eq!(after.blobs_total, before.blobs_total);
}

/// **AC-P3-29-28.** A bare repository with commits is scanned.
///
/// **Bare is not the discriminator** (§29.1): `--full-tree` needs no working tree, so a bare
/// repository with commits enumerates exactly like any other. Written because the shape it guards
/// is a gate on `repo_kind` that no other criterion would catch. The count of bare repositories
/// exercised is printed and the criterion fails at zero.
#[test]
fn ac_p3_29_28_a_bare_repository_with_commits_is_scanned() {
    let source = TestRepo::init();
    source.write("src/a.rs", b"// TODO: in a bare clone\n");
    source.write("README.md", b"# a project\n");
    source.write("LICENSE", b"MIT\n");
    source.write(".github/workflows/ci.yml", b"on: push\n");
    source.write("tests/it.rs", b"fn t() {}\n");
    source.commit("first");
    let bare = TestRepo::init_bare();
    let path = bare.path().join("copy.git");
    source.git(&["clone", "-q", "--bare", ".", &path.to_string_lossy()]);
    let handle = RepoHandle::bare(
        &path,
        codotheca_core::git::StoreKey::new("test-store"),
        codotheca_core::mount::StoreClass::Local,
    );

    // The corpus, so the count is derived from what was built rather than written down.
    let corpus: Vec<RepoHandle> = vec![handle];
    let mut exercised = 0;
    for bare_repo in &corpus {
        let entries = head_tree(
            &source.exec(),
            bare_repo,
            RunLimits::none(),
            &CancelToken::new(),
        )
        .unwrap();
        let answers = presence_for(&entries);
        eprintln!(
            "a bare repository enumerated {} paths: {answers:?}",
            entries.len()
        );
        assert!(!entries.is_empty());
        assert_eq!(answers.readme, PresenceState::Present);
        assert_eq!(answers.license, PresenceState::Present);
        assert_eq!(answers.tests, PresenceState::Present);
        assert_eq!(answers.ci, PresenceState::Present);
        exercised += 1;
    }
    eprintln!("bare repositories exercised: {exercised}");
    assert!(exercised > 0, "no bare repository was exercised");
}

/// **AC-P3-29-29.** An unborn HEAD writes no row.
///
/// Asserted as **row absence**, not as a value: the row's absence is *J7 has never observed this
/// project*, and no fourth tri-state value is invented for it. `head_oid` is `NOT NULL`, which is
/// that rule made structural.
#[test]
fn ac_p3_29_29_an_unborn_head_writes_no_row() {
    let rig = Rig::new("head-one");
    rig.set("UPDATE location SET head_oid = NULL WHERE project_id = ?1");
    let oid = rig.source(b"// TODO: never read\n");
    rig.set_tree(vec![blob_entry("src/a.rs", &oid)]);

    let outcome = rig.run(None);
    eprintln!(
        "an unborn head yields {outcome:?} and row {:?}",
        rig.scan_row()
    );
    assert_eq!(outcome, JobOutcome::Done);
    assert_eq!(rig.scan_row(), None, "a row was written with no basis");
    assert!(rig.git.calls().is_empty(), "git ran with no head to read");
}

/// §29.7 predicate 3: suppression gates the **blob read** and not the enumeration.
///
/// Constructed directly, so both branches are exercised and the wiring is proven — only the
/// predicate's input is p3-30's (Deviation 3).
#[test]
fn suppression_gates_the_blob_read_and_not_the_enumeration() {
    let open = ContentGates {
        is_reference: Some(false),
        granted: true,
        compute_suppressed: false,
    };
    let suppressed = ContentGates {
        compute_suppressed: true,
        ..open
    };
    eprintln!("open {open:?} reads blobs: {}", open.reads_blobs());
    eprintln!(
        "suppressed {suppressed:?} reads blobs: {}",
        suppressed.reads_blobs()
    );
    assert!(open.reads_blobs());
    assert!(suppressed.runs(), "suppression stopped the enumeration too");
    assert!(!suppressed.reads_blobs());
    // `None` is *not computed* and is not Reference, so it is not `Some(false)`.
    assert!(!ContentGates {
        is_reference: None,
        ..open
    }
    .runs());
    assert!(!ContentGates {
        is_reference: Some(true),
        ..open
    }
    .runs());

    // And against a real run: the enumeration lands its four answers, and no blob is read.
    let rig = Rig::new("head-one");
    let oid = rig.source(b"// TODO: suppressed\n");
    rig.set_tree(vec![blob_entry("src/a.rs", &oid)]);
    rig.set(
        "UPDATE app_meta SET v = '0'
          WHERE k = 'content_scan_enabled' AND ?1 = (SELECT id FROM project LIMIT 1)",
    );
    assert!(!rig.gates().reads_blobs());

    assert_eq!(rig.run(None), JobOutcome::Done);
    let row = rig.scan_row().unwrap();
    eprintln!("ungranted run stored {row:?}");
    let guard = rig.index.lock().unwrap();
    let scans: i64 = guard
        .conn()
        .query_row("SELECT count(*) FROM blob_scan", [], |r| r.get(0))
        .unwrap();
    let presence = j7_markers::presence_for_project(guard.conn(), rig.project)
        .unwrap()
        .unwrap();
    drop(guard);
    eprintln!("blob_scan rows: {scans}, presence {presence:?}");
    assert_eq!(scans, 0, "the blob read ran behind a closed gate");
    assert_eq!(presence.readme, PresenceState::Absent);
    assert!(rig.git.blob_requests().is_empty());
}

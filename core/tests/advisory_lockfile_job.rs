//! §32.6's lockfile read has a **production caller**: J6, run by the job runner.
//!
//! The reader had its tests and no real caller, so in the product no project ever got a
//! `project_dependency_scan` row and every dependency verdict sat at *the scan has not run*. These
//! tests drive a real J6 job through `JobRunner` and read what it stored.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::sync::{Arc, Mutex};

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{GitSlots, SystemGit};
use codotheca_core::health::enrolment::{is_enrolled, surface_suppressed};
use codotheca_core::index::Index;
use codotheca_core::jobs::scheduler::JobRunner;
use codotheca_core::jobs::{Job, JobDeps, JobKind, JobOrigin, Priority};
use codotheca_core::mount::StoreClass;
use codotheca_core::proto::EventSink;
use codotheca_core::protocol::{LocationId, ProjectId};
use codotheca_core::testing::FakeClock;
use support::TestRepo;

const T0: i64 = 1_700_000_000;

#[derive(Debug, Default)]
struct SilentSink;

impl EventSink for SilentSink {
    fn emit(&self, _topic: &str, _event: &str, _payload: serde_json::Value) {}
}

fn count(index: &Mutex<Index>, sql: &str, project: ProjectId) -> i64 {
    let guard = index.lock().unwrap();
    guard
        .conn()
        .query_row(sql, [project.0], |r| r.get(0))
        .unwrap()
}

/// One J6 job over `repo`, run to completion by a real `JobRunner`. `repo_kind` is the
/// `location.repo_kind` the scan would have written.
fn run_j6(repo: &TestRepo, repo_kind: &str) -> (Arc<Mutex<Index>>, ProjectId, tempfile::TempDir) {
    run_j6_as(repo, repo_kind, None, false)
}

/// [`run_j6`] over a project with the given enrolment stamp and archive flag.
fn run_j6_as(
    repo: &TestRepo,
    repo_kind: &str,
    acknowledged_at: Option<i64>,
    is_archived: bool,
) -> (Arc<Mutex<Index>>, ProjectId, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let index = Arc::new(Mutex::new(Index::open_at(dir.path(), 0).unwrap()));
    let (project, location) = {
        let guard = index.lock().unwrap();
        let conn = guard.conn();
        conn.execute(
            "INSERT INTO project (name, seed_basename, authored_by_user, acknowledged_at,
                                  is_archived, created_at, updated_at)
             VALUES ('p', 'p', 1, ?1, ?2, 0, 0)",
            rusqlite::params![acknowledged_at, i64::from(is_archived)],
        )
        .unwrap();
        let project = ProjectId(conn.last_insert_rowid());
        let path = repo.path().to_string_lossy().into_owned();
        conn.execute(
            "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                   store_key, presence, repo_kind)
             VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', ?4)",
            rusqlite::params![project.0, path.as_bytes(), path, repo_kind],
        )
        .unwrap();
        let location = LocationId(conn.last_insert_rowid());
        drop(guard);
        (project, location)
    };

    let clock = Arc::new(FakeClock::new(T0));
    let deps = JobDeps {
        git: Arc::new(SystemGit::new(
            Arc::new(repo.exec()),
            Arc::new(GitSlots::new(4)),
            clock.clone(),
        )),
        clock,
        cancel: CancelToken::new(),
        tz_offset_min: 0,
    };
    let runner = JobRunner::new(Arc::clone(&index), deps, Arc::new(SilentSink));
    assert!(runner.enqueue(Job {
        kind: JobKind::J6Content,
        project_id: project,
        location_id: location,
        store_key: "store".to_owned(),
        store_kind: StoreClass::Local,
        priority: Priority::Interactive,
        not_before: 0,
        origin: JobOrigin::Interactive,
    }));
    runner.start(1);

    // J6 writes its `peek_cache` row in the same transaction as the lockfile rows, so the row
    // appearing is the job having written. Spun rather than slept.
    let done = "SELECT count(*) FROM peek_cache WHERE project_id = ?1";
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while std::time::Instant::now() < deadline && count(&index, done, project) == 0 {
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    runner.request_stop();
    runner.join();
    assert_eq!(count(&index, done, project), 1, "J6 never wrote");
    (index, project, dir)
}

/// A J6 run over a working copy holding two lockfiles, one at depth 2, writes the scan row, both
/// lockfile rows and their triples.
#[test]
fn a_j6_run_reads_the_lockfiles_of_the_working_copy() {
    let repo = TestRepo::init();
    repo.write(
        "Cargo.lock",
        b"[[package]]\nname = \"alpha\"\nversion = \"1.0.0\"\n",
    );
    repo.write(
        "tools/Cargo.lock",
        b"[[package]]\nname = \"beta\"\nversion = \"2.0.0\"\n",
    );
    let (index, project, _dir) = run_j6(&repo, "worktree");

    let scans = count(
        &index,
        "SELECT count(*) FROM project_dependency_scan WHERE project_id = ?1",
        project,
    );
    let files = count(
        &index,
        "SELECT count(*) FROM project_lockfile WHERE project_id = ?1 AND read_state = 'parsed'",
        project,
    );
    let triples: Vec<(String, String)> = {
        let guard = index.lock().unwrap();
        let mut stmt = guard
            .conn()
            .prepare(
                "SELECT package_name, source_path FROM project_dependency
                  WHERE project_id = ?1 ORDER BY package_name",
            )
            .unwrap();
        let rows = stmt
            .query_map([project.0], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        drop(stmt);
        drop(guard);
        rows
    };
    eprintln!(
        "advisory_lockfile_job: {scans} scan row(s), {files} parsed lockfile(s), triples {triples:?}"
    );
    assert_eq!(scans, 1, "J6 ran and recorded no lockfile scan");
    assert_eq!(files, 2);
    assert_eq!(
        triples,
        vec![
            ("alpha".to_owned(), "Cargo.lock".to_owned()),
            ("beta".to_owned(), "tools/Cargo.lock".to_owned()),
        ]
    );
}

/// A bare repository has no working copy, so J6 records no scan: an empty walk of its git
/// directory would read as *ran and found nothing* and render a clean verdict.
#[test]
fn a_j6_run_over_a_bare_repository_records_no_lockfile_scan() {
    let repo = TestRepo::init_bare();
    let (index, project, _dir) = run_j6(&repo, "bare");
    assert_eq!(
        count(
            &index,
            "SELECT count(*) FROM project_dependency_scan WHERE project_id = ?1",
            project,
        ),
        0,
        "a bare repository's scan row claims a dependency set nobody read"
    );
}

/// **AC-P3-30-11, §32's half.** A `surface_suppressed` project's lockfile read still runs and
/// writes its rows — unenrolled, and enrolled but archived — because the read is bounded and
/// `surface_suppressed` gates rendering, ranking and notification, not computation.
///
/// Both sides are printed: the rows the read wrote, and the blob read that stays closed.
#[test]
fn ac_p3_30_11_a_suppressed_projects_lockfiles_are_still_read() {
    let mut cases = 0usize;
    for (acknowledged_at, is_archived) in [(None, false), (Some(T0), true)] {
        let repo = TestRepo::init();
        repo.write(
            "Cargo.lock",
            b"[[package]]\nname = \"alpha\"\nversion = \"1.0.0\"\n",
        );
        let (index, project, _dir) = run_j6_as(&repo, "worktree", acknowledged_at, is_archived);
        assert!(
            surface_suppressed(is_enrolled(acknowledged_at), is_archived),
            "the fixture is not surface-suppressed"
        );

        let scans = count(
            &index,
            "SELECT count(*) FROM project_dependency_scan WHERE project_id = ?1",
            project,
        );
        let lockfiles = count(
            &index,
            "SELECT count(*) FROM project_lockfile WHERE project_id = ?1",
            project,
        );
        let triples = count(
            &index,
            "SELECT count(*) FROM project_dependency WHERE project_id = ?1",
            project,
        );
        // `blob_scan` is content-addressed and carries no project; this index holds one project.
        let blobs: i64 = index
            .lock()
            .unwrap()
            .conn()
            .query_row("SELECT count(*) FROM blob_scan", [], |r| r.get(0))
            .unwrap();
        eprintln!(
            "suppressed (acknowledged_at={acknowledged_at:?}, archived={is_archived}): running side \
             scan={scans} lockfile={lockfiles} dependency={triples}; blob_scan={blobs}"
        );
        assert!(
            scans > 0 && lockfiles > 0 && triples > 0,
            "a surface-suppressed project's lockfile read wrote nothing"
        );
        assert_eq!(blobs, 0);
        cases += 1;
    }
    assert_eq!(cases, 2);
}

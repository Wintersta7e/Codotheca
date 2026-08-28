//! `project_job_state` against the real schema.
//!
//! The point of this file is that it reads the *other side*: every value the Rust vocabulary
//! emits is inserted into the column that stores it, so a CHECK constraint and an enum cannot
//! drift the way R26 found them drifting.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::must_use_candidate,
    clippy::missing_panics_doc
)]

use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::jobs::state::{
    coverage_for, load, put, reset_for, JobCoverage, JobStateRow, ResetCause,
};
use codotheca_core::jobs::{JobKind, JobState};
use codotheca_core::protocol::ProjectId;

pub fn fresh() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

pub fn insert_project(conn: &rusqlite::Connection, name: &str) -> ProjectId {
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES (?1, ?1, 0, 0)",
        [name],
    )
    .unwrap();
    ProjectId(conn.last_insert_rowid())
}

/// R26's shape, held mechanically: the DDL's CHECK and `JobState::slug` are one value stated
/// twice. `"done"` — the spelling plan 09's body uses — is rejected here, which is why the
/// enum spells it `"ok"`.
#[test]
fn every_job_state_slug_is_accepted_by_the_column() {
    let (_dir, mut conn) = fresh();
    let project = insert_project(&conn, "p");
    for (i, state) in JobState::ALL.into_iter().enumerate() {
        let job = JobKind::ALL[i % JobKind::ALL.len()];
        let tx = conn.transaction().unwrap();
        put(&tx, project, &JobStateRow::fresh(job, state, 10)).unwrap();
        tx.commit()
            .unwrap_or_else(|e| panic!("state {:?} ({}) was refused: {e}", state, state.slug()));
    }
}

/// The same for the `job` column, whose CHECK lists `j0` and `j5` this scheduler never queues.
#[test]
fn every_job_slug_is_accepted_by_the_column() {
    let (_dir, mut conn) = fresh();
    let project = insert_project(&conn, "p");
    for job in JobKind::ALL {
        let tx = conn.transaction().unwrap();
        put(&tx, project, &JobStateRow::fresh(job, JobState::Queued, 1)).unwrap();
        tx.commit()
            .unwrap_or_else(|e| panic!("job {} was refused: {e}", job.slug()));
    }
}

#[test]
fn a_row_round_trips_through_the_database() {
    let (_dir, mut conn) = fresh();
    let project = insert_project(&conn, "p");
    let mut row = JobStateRow::fresh(JobKind::J4History, JobState::Queued, 99);
    row.cursor = Some("12000".to_owned());
    row.progress_done = Some(400);
    row.progress_total = Some(9000);
    row.fail_count = 2;
    row.reason = Some("lock".to_owned());

    let tx = conn.transaction().unwrap();
    put(&tx, project, &row).unwrap();
    tx.commit().unwrap();

    let back = load(&conn, project).unwrap();
    assert_eq!(back, vec![row]);
}

/// `put` is an upsert on `(project_id, job)`; a second write must replace, not duplicate.
#[test]
fn a_second_write_for_the_same_job_replaces_the_row() {
    let (_dir, mut conn) = fresh();
    let project = insert_project(&conn, "p");

    let tx = conn.transaction().unwrap();
    put(
        &tx,
        project,
        &JobStateRow::fresh(JobKind::J1Refstate, JobState::Queued, 1),
    )
    .unwrap();
    put(
        &tx,
        project,
        &JobStateRow::fresh(JobKind::J1Refstate, JobState::Done, 2),
    )
    .unwrap();
    tx.commit().unwrap();

    let back = load(&conn, project).unwrap();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].state, JobState::Done);
    assert_eq!(back[0].at, 2);
}

/// A row written by a newer build is skipped, not guessed at.
///
/// **This test's example changed when plan 10 added `J5Art`.** It used `j5` as "a real schema
/// job this scheduler does not queue"; `j5` is now queued, so that premise is gone and every
/// slug the column's CHECK permits maps to a `JobKind`. The guarantee itself still matters —
/// it protects this build against a row a *later* migration's vocabulary wrote — so the row is
/// now inserted with check constraints suspended, which is exactly the state a newer build
/// that widened the CHECK would leave behind.
#[test]
fn an_unknown_job_slug_is_skipped_rather_than_guessed() {
    let (_dir, conn) = fresh();
    let project = insert_project(&conn, "p");
    conn.execute_batch("PRAGMA ignore_check_constraints = ON")
        .unwrap();
    conn.execute(
        "INSERT INTO project_job_state (project_id, job, state, at) VALUES (?1, 'j7', 'queued', 0)",
        [project.0],
    )
    .unwrap();
    conn.execute_batch("PRAGMA ignore_check_constraints = OFF")
        .unwrap();
    assert_eq!(JobKind::from_slug("j7"), None);
    assert!(load(&conn, project).unwrap().is_empty());
}

/// The other half, and the one plan 10 makes newly true: every slug the column permits is a
/// slug this build can name. R34's mirror, read from the column side.
#[test]
fn every_slug_the_column_permits_is_a_job_kind_this_build_knows() {
    let (_dir, conn) = fresh();
    let project = insert_project(&conn, "p");
    for slug in ["j1", "j1_5", "j2", "j3", "j4", "j5", "j6"] {
        conn.execute(
            "INSERT INTO project_job_state (project_id, job, state, at)
             VALUES (?1, ?2, 'queued', 0)",
            rusqlite::params![project.0, slug],
        )
        .unwrap();
        assert!(
            JobKind::from_slug(slug).is_some(),
            "{slug} is stored by the column and named by no JobKind"
        );
    }
    assert_eq!(load(&conn, project).unwrap().len(), 7);
}

/// `reset_for` is plan 08 Task 17's dependency: a merge and its requeue must commit together,
/// which is why it takes the caller's transaction rather than opening its own.
#[test]
fn a_reset_revives_only_the_stuck_rows_and_records_why() {
    let (_dir, mut conn) = fresh();
    let project = insert_project(&conn, "p");

    let tx = conn.transaction().unwrap();
    for (job, state) in [
        (JobKind::J1Refstate, JobState::DeferredSlow),
        (JobKind::J2Status, JobState::Failed),
        (JobKind::J3Inventory, JobState::Done),
        (JobKind::J4History, JobState::Queued),
    ] {
        let mut row = JobStateRow::fresh(job, state, 1);
        row.fail_count = 3;
        put(&tx, project, &row).unwrap();
    }
    tx.commit().unwrap();

    let tx = conn.transaction().unwrap();
    let n = reset_for(&tx, project, ResetCause::UserRequested, 500).unwrap();
    tx.commit().unwrap();
    assert_eq!(n, 2, "only deferred_slow and failed are revived");

    let rows = load(&conn, project).unwrap();
    let by = |k: JobKind| rows.iter().find(|r| r.job == k).unwrap().clone();

    for k in [JobKind::J1Refstate, JobKind::J2Status] {
        let r = by(k);
        assert_eq!(r.state, JobState::Queued);
        assert_eq!(r.fail_count, 0);
        assert_eq!(r.reason.as_deref(), Some("user_requested"));
        assert_eq!(r.at, 500);
    }
    // A finished job is not re-run and a queued one is not disturbed.
    assert_eq!(by(JobKind::J3Inventory).state, JobState::Done);
    assert_eq!(by(JobKind::J4History).at, 1);
}

/// A reset inside a transaction that rolls back must leave the rows stuck — otherwise plan 08's
/// "merge and requeue commit together" is not true.
#[test]
fn a_rolled_back_reset_leaves_the_rows_deferred() {
    let (_dir, mut conn) = fresh();
    let project = insert_project(&conn, "p");

    let tx = conn.transaction().unwrap();
    put(
        &tx,
        project,
        &JobStateRow::fresh(JobKind::J1Refstate, JobState::DeferredSlow, 1),
    )
    .unwrap();
    tx.commit().unwrap();

    let tx = conn.transaction().unwrap();
    reset_for(&tx, project, ResetCause::StoreReturned, 7).unwrap();
    tx.rollback().unwrap();

    assert_eq!(
        load(&conn, project).unwrap()[0].state,
        JobState::DeferredSlow
    );
}

/// §8.2's coverage caption reads finished jobs, never queued ones.
#[test]
fn coverage_counts_only_finished_jobs() {
    let (_dir, mut conn) = fresh();
    let project = insert_project(&conn, "p");
    assert_eq!(
        coverage_for(&conn, project).unwrap(),
        JobCoverage {
            has_j1_result: false,
            inventory_done: false
        }
    );

    let tx = conn.transaction().unwrap();
    put(
        &tx,
        project,
        &JobStateRow::fresh(JobKind::J1Refstate, JobState::Done, 1),
    )
    .unwrap();
    put(
        &tx,
        project,
        &JobStateRow::fresh(JobKind::J3Inventory, JobState::Queued, 1),
    )
    .unwrap();
    tx.commit().unwrap();

    assert_eq!(
        coverage_for(&conn, project).unwrap(),
        JobCoverage {
            has_j1_result: true,
            inventory_done: false
        }
    );
}

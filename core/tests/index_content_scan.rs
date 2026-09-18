//! `0012_content_scan.sql` — the `project_job_state` rebuild, and the three tables §29 adds.
//!
//! The rebuild is the dangerous half. `progress_done` and `progress_total` were added by `ALTER`
//! in `0007_jobs_derived.sql:4-5`, **after** `0005` declared the table, so a rebuild drafted from
//! `0005`'s text alone silently drops both — and criterion 20's coverage indicator with them.
//! `idx_job_state_ready` (`0007:14`) goes the same way: it hangs off the dropped table.
//!
//! Every check here reads the *other* side — a real migrated database — rather than the SQL text,
//! except the one that is about the text.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::index::migrate::{apply_all, MIGRATIONS, SUPPORTED_SCHEMA_VERSION};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::jobs::content_scan::BlobOutcome;
use codotheca_core::jobs::markers::Marker;
use codotheca_core::jobs::presence::{PresenceState, PREDICATE_VERSION};

/// The migration's own text, for the one assertion that is about the file rather than the store.
const MIGRATION_SQL: &str = include_str!("../migrations/0012_content_scan.sql");

fn scratch() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_connection(&Index::db_path(dir.path())).unwrap();
    (dir, conn)
}

/// A database at exactly `version`, through the shipped migration set.
fn migrated_to(version: usize) -> (tempfile::TempDir, rusqlite::Connection) {
    let (dir, mut conn) = scratch();
    apply_all(&mut conn, &MIGRATIONS[..version]).unwrap();
    (dir, conn)
}

fn columns(conn: &rusqlite::Connection, table: &str) -> Vec<String> {
    let mut statement = conn
        .prepare(&format!("PRAGMA table_info(\"{table}\")"))
        .unwrap();
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap();
    rows.map(Result::unwrap).collect()
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

/// **AC-P3-29-22.** The rebuild carries every column and every value.
///
/// A row written by the shipped build — cursor, `progress_done`, `progress_total` all non-NULL —
/// survives the rebuild with its values intact. Drafting the new table from `0005`'s text alone
/// makes this fail with `no such column: progress_done` on the `INSERT INTO … SELECT`.
#[test]
fn ac_p3_29_22_the_rebuild_carries_every_column_and_every_value() {
    let (_dir, mut conn) = migrated_to(11);
    let project = insert_project(&conn, "p");
    conn.execute(
        "INSERT INTO project_job_state
           (project_id, job, state, fail_count, reason, at, cursor, progress_done, progress_total)
         VALUES (?1, 'j4', 'running', 2, 'why', 1700, 'abc', 41, 99)",
        [project],
    )
    .unwrap();

    apply_all(&mut conn, MIGRATIONS).unwrap();

    let found = columns(&conn, "project_job_state");
    eprintln!("project_job_state columns after the rebuild: {found:?}");
    assert!(!found.is_empty(), "read no columns at all");
    for wanted in ["cursor", "progress_done", "progress_total"] {
        assert!(
            found.iter().any(|c| c == wanted),
            "the rebuild dropped `{wanted}`; columns are {found:?}"
        );
    }

    let (cursor, done, total, fail_count, reason, at): (
        Option<String>,
        Option<i64>,
        Option<i64>,
        i64,
        Option<String>,
        i64,
    ) = conn
        .query_row(
            "SELECT cursor, progress_done, progress_total, fail_count, reason, at
               FROM project_job_state WHERE project_id = ?1 AND job = 'j4'",
            [project],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(cursor.as_deref(), Some("abc"));
    assert_eq!(done, Some(41));
    assert_eq!(total, Some(99));
    assert_eq!(fail_count, 2);
    assert_eq!(reason.as_deref(), Some("why"));
    assert_eq!(at, 1700);
}

/// R59, read off the file rather than off the store, because the defect is textual: a
/// `PRAGMA foreign_keys=OFF` written into a migration is a documented no-op inside the
/// transaction `apply_all` wraps it in, and the toggle lives in the runner.
#[test]
fn no_pragma_appears_in_the_migration() {
    eprintln!(
        "read {} bytes of 0012_content_scan.sql",
        MIGRATION_SQL.len()
    );
    assert!(!MIGRATION_SQL.is_empty(), "read an empty migration file");
    // The rule is about statements, and the file's own comment explains R59 by naming the pragma
    // it must not execute — so the comments come off first.
    let statements: String = MIGRATION_SQL
        .lines()
        .map(|line| line.split("--").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    eprintln!(
        "{} bytes of statement text after comments",
        statements.len()
    );
    assert!(!statements.trim().is_empty(), "stripped every byte");
    assert!(
        !statements.to_uppercase().contains("PRAGMA"),
        "0012_content_scan.sql executes a PRAGMA (R59)"
    );
}

/// `idx_job_state_ready` is on the rebuilt table and is declared in `0007`, not `0005`, so a
/// rebuild drafted from `0005` drops it and recreates nothing.
#[test]
fn a_bare_rebuild_would_have_dropped_the_index() {
    let (_dir, mut conn) = scratch();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master
              WHERE type='index' AND name='idx_job_state_ready'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    eprintln!("idx_job_state_ready rows in sqlite_master: {count}");
    assert_eq!(count, 1, "the rebuild dropped idx_job_state_ready");
}

/// The three tables §29.3 and §29.5 declare, against a migrated database.
#[test]
fn the_migration_lands_three_tables_and_the_version() {
    let (_dir, mut conn) = scratch();
    let reached = apply_all(&mut conn, MIGRATIONS).unwrap();
    assert_eq!(reached, SUPPORTED_SCHEMA_VERSION);
    let mut seen = 0;
    for table in ["blob_scan", "blob_finding", "project_content_scan"] {
        let found: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(found, 1, "{table} was not created");
        seen += 1;
    }
    eprintln!("tables checked: {seen}");
    assert!(seen > 0, "checked no tables");
}

/// `blob_scan` and `blob_finding` are library-wide and content-addressed: **neither carries a
/// `project_id`**, because a blob's content is not a property of any project (§29.12.3).
#[test]
fn the_blob_cache_carries_no_project_id() {
    let (_dir, mut conn) = scratch();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    for table in ["blob_scan", "blob_finding"] {
        let found = columns(&conn, table);
        eprintln!("{table} columns: {found:?}");
        assert!(!found.is_empty(), "{table} has no columns");
        assert!(
            !found.iter().any(|c| c == "project_id"),
            "{table} grew a project_id"
        );
    }
}

/// `head_oid` is `NOT NULL` — the unborn-HEAD rule made structural (§29.5). A project with no
/// HEAD tree cannot have a row, so the rule cannot be forgotten by a later writer.
#[test]
fn a_content_scan_row_cannot_exist_without_a_head() {
    let (_dir, mut conn) = scratch();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    let project = insert_project(&conn, "p");
    let refused = conn.execute(
        "INSERT INTO project_content_scan
           (project_id, head_oid, predicate_version, has_readme, has_license, has_tests, has_ci,
            presence_observed_at, enumerated_at)
         VALUES (?1, NULL, 1, 'present', 'present', 'present', 'present', 0, 0)",
        [project],
    );
    assert!(refused.is_err(), "a NULL head_oid was accepted");
}

/// R26's shape, read from the column side: a DDL CHECK rejecting the values its own core emits.
/// Every `PresenceState` slug is inserted against a real migrated database, into all four
/// columns, and **the number inserted is printed — a run that inserts zero is a failing run.**
#[test]
fn every_presence_slug_is_accepted_by_the_column() {
    let (_dir, mut conn) = scratch();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    let slugs: Vec<&'static str> = PresenceState::ALL.iter().map(|s| s.slug()).collect();
    eprintln!(
        "PresenceState::ALL derives {} slugs: {slugs:?}",
        slugs.len()
    );

    let mut inserted = 0;
    for (n, slug) in slugs.iter().enumerate() {
        let project = insert_project(&conn, &format!("p{n}"));
        conn.execute(
            "INSERT INTO project_content_scan
               (project_id, head_oid, predicate_version, has_readme, has_license, has_tests,
                has_ci, presence_observed_at, enumerated_at)
             VALUES (?1, 'deadbeef', ?2, ?3, ?3, ?3, ?3, 0, 0)",
            rusqlite::params![project, PREDICATE_VERSION, slug],
        )
        .unwrap_or_else(|e| panic!("the column refused {slug}: {e}"));
        inserted += 1;
        assert_eq!(PresenceState::from_slug(slug), Some(PresenceState::ALL[n]));
    }
    eprintln!("presence slugs accepted by project_content_scan: {inserted}");
    assert!(
        inserted > 0,
        "inserted nothing, so the column proved nothing"
    );
}

/// R26 again, one level down: `blob_scan.outcome`'s CHECK against every slug `BlobOutcome`
/// emits, and `blob_finding.marker`'s against every slug `Marker` emits. **Both print the number
/// inserted and fail at zero** — a loop over an empty vocabulary would pass while looking at
/// nothing.
#[test]
fn every_blob_outcome_slug_is_accepted_by_the_column() {
    let (_dir, mut conn) = scratch();
    apply_all(&mut conn, MIGRATIONS).unwrap();

    let outcomes: Vec<&'static str> = BlobOutcome::ALL.iter().map(|o| o.slug()).collect();
    eprintln!(
        "BlobOutcome::ALL derives {} slugs: {outcomes:?}",
        outcomes.len()
    );
    let mut inserted = 0;
    for (n, slug) in outcomes.iter().enumerate() {
        conn.execute(
            "INSERT INTO blob_scan (blob_oid, scanner_version, outcome, size_bytes, scanned_at)
             VALUES (?1, 1, ?2, 0, 0)",
            rusqlite::params![format!("{n:040}"), slug],
        )
        .unwrap_or_else(|e| panic!("blob_scan.outcome refused {slug}: {e}"));
        inserted += 1;
        assert_eq!(BlobOutcome::from_slug(slug), Some(BlobOutcome::ALL[n]));
    }
    eprintln!("outcome slugs accepted by blob_scan: {inserted}");
    assert!(
        inserted > 0,
        "inserted nothing, so the column proved nothing"
    );

    let markers: Vec<&'static str> = Marker::ALL.iter().map(|m| m.slug()).collect();
    eprintln!("Marker::ALL derives {} slugs: {markers:?}", markers.len());
    let mut filed = 0;
    for (n, slug) in markers.iter().enumerate() {
        conn.execute(
            "INSERT INTO blob_finding
               (blob_oid, scanner_version, ordinal_in_blob, marker, salient_sha256,
                salient_text_capped, line, \"column\")
             VALUES (?1, 1, ?2, ?3, 'sha', 'text', 1, 1)",
            rusqlite::params![format!("{:040}", 0), n, slug],
        )
        .unwrap_or_else(|e| panic!("blob_finding.marker refused {slug}: {e}"));
        filed += 1;
        assert_eq!(Marker::from_slug(slug), Some(Marker::ALL[n]));
    }
    eprintln!("marker slugs accepted by blob_finding: {filed}");
    assert!(filed > 0, "inserted nothing, so the column proved nothing");
}

/// The findings are children of the read that produced them: deleting the `blob_scan` row takes
/// its occurrences with it, so an occurrence set can never outlive the record of its read.
#[test]
fn a_finding_cannot_outlive_the_scan_row_it_hangs_off() {
    let (_dir, mut conn) = scratch();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    conn.execute("PRAGMA foreign_keys = ON", []).unwrap();
    let oid = "f".repeat(40);
    conn.execute(
        "INSERT INTO blob_scan (blob_oid, scanner_version, outcome, size_bytes, scanned_at)
         VALUES (?1, 1, 'scanned', 10, 0)",
        [&oid],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO blob_finding
           (blob_oid, scanner_version, ordinal_in_blob, marker, salient_sha256,
            salient_text_capped, line, \"column\")
         VALUES (?1, 1, 0, 'TODO', 'sha', 'TODO x', 1, 1)",
        [&oid],
    )
    .unwrap();
    conn.execute("DELETE FROM blob_scan WHERE blob_oid = ?1", [&oid])
        .unwrap();
    let left: i64 = conn
        .query_row("SELECT count(*) FROM blob_finding", [], |row| row.get(0))
        .unwrap();
    eprintln!("findings left after the parent went: {left}");
    assert_eq!(left, 0);
}

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §32.16's merge: **three tables recomputed, one latched ledger reparented.**
//!
//! A14.4's three classes, in A14.4's vocabulary — `derived`, `not-recomputable`, `reparented`. A
//! table in the wrong class either loses earned XP or re-notifies the user for an advisory they
//! were already told about, on the one surface they cannot dismiss before reading.

use codotheca_core::identity::merge::{recompute_derived, reparent_rows};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};

const NOW: i64 = 1_800_000_000;

fn fresh() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

fn project(conn: &rusqlite::Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES (?1, ?1, 1, 1)",
        [name],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn seed_read(conn: &rusqlite::Connection, project: i64, package: &str) {
    conn.execute(
        "INSERT INTO project_dependency_scan
           (project_id, observed_at, files_matched, dirs_entered, unresolved_manifests, complete)
         VALUES (?1, ?2, 1, 1, 0, 1)",
        rusqlite::params![project, NOW],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO project_lockfile
           (project_id, source_path, ecosystem, read_state, size_bytes, observed_at)
         VALUES (?1, 'Cargo.lock', 'rust', 'parsed', 10, ?2)",
        rusqlite::params![project, NOW],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO project_dependency
           (project_id, ecosystem, package_name, version, source_path, observed_at)
         VALUES (?1, 'rust', ?2, '1.0.0', 'Cargo.lock', ?3)",
        rusqlite::params![project, package, NOW],
    )
    .unwrap();
}

fn seed_notified(conn: &rusqlite::Connection, project: i64, advisory: &str) {
    conn.execute(
        "INSERT INTO advisory_notified (project_id, advisory_id, at, seeded)
         VALUES (?1, ?2, ?3, 0)",
        rusqlite::params![project, advisory, NOW],
    )
    .unwrap();
}

/// The library-wide cache, which no merge may touch.
fn seed_library_wide(conn: &rusqlite::Connection) {
    conn.execute(
        "INSERT INTO advisory_sweep (started_at, settled_at, outcome, resource, complete)
         VALUES (?1, ?1, 'done', 'core', 1)",
        [NOW],
    )
    .unwrap();
    let sweep = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO advisory (advisory_id, severity, summary, url, observed_at)
         VALUES ('GHSA-lib', 'high', 's', 'u', ?1)",
        [NOW],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO advisory_cve (advisory_id, cve_id) VALUES ('GHSA-lib', 'CVE-1')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO advisory_triple
           (ecosystem, package_name, version, sweep_id, observed_at, answered)
         VALUES ('rust', 'left', '1.0.0', ?1, ?2, 1)",
        rusqlite::params![sweep, NOW],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO advisory_match
           (ecosystem, package_name, version, advisory_id, fix_available, fixed_version)
         VALUES ('rust', 'left', '1.0.0', 'GHSA-lib', 1, '2.0.0')",
        [],
    )
    .unwrap();
}

fn count(conn: &rusqlite::Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn pairs(conn: &rusqlite::Connection) -> Vec<(i64, String)> {
    let mut stmt = conn
        .prepare("SELECT project_id, advisory_id FROM advisory_notified ORDER BY 1, 2")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// **AC-P3-32-21.** The three read tables are **deleted for both sides**; `advisory_notified` is
/// **reparented**; and a pair held by **both** sides does not collide, leaving the survivor not
/// re-notified.
#[test]
fn ac_p3_32_21_the_notice_ledger_is_carried_across_a_merge() {
    let (_d, mut conn) = fresh();
    let survivor = project(&conn, "survivor");
    let absorbed = project(&conn, "absorbed");
    seed_read(&conn, survivor, "left");
    seed_read(&conn, absorbed, "right");
    seed_library_wide(&conn);

    // The collision: both sides were told about `GHSA-both`.
    seed_notified(&conn, survivor, "GHSA-both");
    seed_notified(&conn, absorbed, "GHSA-both");
    seed_notified(&conn, absorbed, "GHSA-only-absorbed");

    let tx = conn.transaction().unwrap();
    let counts = reparent_rows(&tx, survivor, absorbed).expect("reparent");
    recompute_derived(&tx, survivor, absorbed).expect("recompute");
    tx.commit().unwrap();

    eprintln!(
        "advisory_merge: {} advisory_notified row(s) reparented",
        counts.advisory_notified
    );
    assert_eq!(
        counts.advisory_notified, 1,
        "the collided pair was dropped, the other moved"
    );
    assert_eq!(
        pairs(&conn),
        vec![
            (survivor, "GHSA-both".to_owned()),
            (survivor, "GHSA-only-absorbed".to_owned()),
        ],
        "one row per pair, on the survivor, and the survivor is not re-notified"
    );

    // Derived: deleted for **both** sides, so the recompute opens the survivor's rows fresh.
    assert_eq!(count(&conn, "SELECT count(*) FROM project_dependency"), 0);
    assert_eq!(count(&conn, "SELECT count(*) FROM project_lockfile"), 0);
    assert_eq!(
        count(&conn, "SELECT count(*) FROM project_dependency_scan"),
        0
    );
}

/// A **stale** row for the survivor does not survive the merge. Deleting both sides is required,
/// not tidy: a row kept for the survivor would be read as current against a worktree nobody
/// re-walked.
#[test]
fn a_stale_survivor_row_does_not_survive() {
    let (_d, mut conn) = fresh();
    let survivor = project(&conn, "survivor");
    let absorbed = project(&conn, "absorbed");
    seed_read(&conn, survivor, "stale");

    let tx = conn.transaction().unwrap();
    recompute_derived(&tx, survivor, absorbed).expect("recompute");
    tx.commit().unwrap();

    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM project_dependency WHERE package_name = 'stale'"
        ),
        0
    );
}

/// The five library-wide tables are **byte-identical** before and after: a merge does not touch a
/// table with no `project_id`, and two project rows merging cannot have invalidated what a third
/// party published.
#[test]
fn a_merge_does_not_touch_the_library_wide_cache() {
    let (_d, mut conn) = fresh();
    let survivor = project(&conn, "survivor");
    let absorbed = project(&conn, "absorbed");
    seed_library_wide(&conn);

    let snapshot = |conn: &rusqlite::Connection| -> Vec<(String, i64)> {
        [
            "advisory_sweep",
            "advisory",
            "advisory_cve",
            "advisory_triple",
            "advisory_match",
        ]
        .into_iter()
        .map(|t| {
            (
                t.to_owned(),
                count(conn, &format!("SELECT count(*) FROM {t}")),
            )
        })
        .collect()
    };
    let before = snapshot(&conn);
    assert!(
        before.iter().all(|(_, n)| *n > 0),
        "a snapshot of five empty tables proves nothing: {before:?}"
    );

    let tx = conn.transaction().unwrap();
    reparent_rows(&tx, survivor, absorbed).expect("reparent");
    recompute_derived(&tx, survivor, absorbed).expect("recompute");
    tx.commit().unwrap();

    eprintln!("advisory_merge: library-wide rows before={before:?}");
    assert_eq!(snapshot(&conn), before);
}

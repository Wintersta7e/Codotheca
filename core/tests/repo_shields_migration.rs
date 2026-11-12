//! `0025_repo_shields.sql` — the licence id and the newest tag (§55.5): four nullable columns added
//! in place, no table rebuilt, every existing row keeping its `rowid` and reading NULL.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;

use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use rusqlite::{params, Connection};

/// The version before this migration: the shipped chain up to and including `0024`.
const BEFORE: usize = 24;

/// The four columns §55.5 adds, by table.
const ADDED: [(&str, &str); 4] = [
    ("project_content_scan", "license_id"),
    ("project_content_scan", "license_id_head_oid"),
    ("location", "latest_tag_name"),
    ("location", "latest_tag_basis"),
];

/// A database in `dir` at exactly `version` files applied, through the shipped set.
fn migrated_to(dir: &Path, version: usize) -> Connection {
    let mut conn = open_connection(&Index::db_path(dir)).unwrap();
    apply_all(&mut conn, &MIGRATIONS[..version]).unwrap();
    conn
}

/// The column names `table` holds, read in the test and never in the migration.
fn columns(conn: &Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// The `CREATE TABLE` text up to its first column, which a rebuild would rewrite.
fn ddl_prefix(conn: &Connection, table: &str) -> String {
    let sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table],
            |r| r.get(0),
        )
        .unwrap();
    sql.split_once('(').unwrap().0.to_owned()
}

fn rowid(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT rowid FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[test]
fn ac_p4_55_12_0025_adds_four_columns_and_rebuilds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = migrated_to(dir.path(), BEFORE);
    conn.execute(
        "INSERT INTO project (name, seed_basename, last_touched_at, created_at, updated_at)
         VALUES ('p', 'p', 1, 1, 1)",
        [],
    )
    .unwrap();
    let project = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO location (project_id, kind, distro, path_bytes, path_key,
                               path_display, volume_key, store_key, presence, repo_kind)
         VALUES (?1, 'linux', '', CAST('/p' AS BLOB), CAST('/p' AS BLOB), '/p', 'v', 's',
                 'present', 'worktree')",
        params![project],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO project_content_scan (project_id, head_oid, predicate_version, has_readme,
                                           has_license, has_tests, has_ci,
                                           presence_observed_at, enumerated_at)
         VALUES (?1, 'h', 1, 'present', 'present', 'absent', 'present', 1, 1)",
        params![project],
    )
    .unwrap();

    let tables = ["project", "location", "project_content_scan"];
    let before: Vec<(i64, String)> = tables
        .iter()
        .map(|t| (rowid(&conn, t), ddl_prefix(&conn, t)))
        .collect();
    for (table, column) in ADDED {
        assert!(
            !columns(&conn, table).iter().any(|c| c == column),
            "{table}.{column} exists before 0025"
        );
    }

    // Through `0025` only: a later file steps the version past the one this test asserts.
    apply_all(&mut conn, &MIGRATIONS[..=BEFORE]).unwrap();

    for (table, column) in ADDED {
        let names = columns(&conn, table);
        eprintln!("repo_shields_migration: {table} columns {names:?}");
        assert!(
            names.iter().any(|c| c == column),
            "{table}.{column} missing"
        );
        let null: bool = conn
            .query_row(&format!("SELECT {column} IS NULL FROM {table}"), [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(
            null,
            "{table}.{column} is not NULL on a row written before it"
        );
    }
    for (table, (id, prefix)) in tables.iter().zip(&before) {
        let count: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        eprintln!("repo_shields_migration: {table} holds {count} row(s)");
        assert_eq!(count, 1, "{table} lost or gained a row");
        assert_eq!(rowid(&conn, table), *id, "{table}'s rowid moved");
        assert_eq!(&ddl_prefix(&conn, table), prefix, "{table} was rebuilt");
    }

    let migration = &MIGRATIONS[BEFORE];
    assert_eq!(migration.version, 25);
    assert_eq!(migration.name, "repo_shields");
    assert!(!migration.rebuilds_a_table);
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 25);
}

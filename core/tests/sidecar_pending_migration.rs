//! `0017_sidecar_pending.sql` — the table a rebuilt index stages its subject records in until the
//! scan brings each project back (§48.8.4), read off a real migrated database.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use rusqlite::Connection;

fn fresh() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

/// The six columns, the subject index, and **no foreign key**: a pending record names a subject no
/// project holds yet, so a reference to `project` would refuse the very rows the table exists for.
#[test]
fn the_pending_table_has_six_columns_one_index_and_no_foreign_key() {
    let (_dir, conn) = fresh();

    let mut stmt = conn
        .prepare("SELECT name, type, \"notnull\", pk FROM pragma_table_info('sidecar_pending')")
        .unwrap();
    let columns: Vec<(String, String, bool, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    eprintln!("sidecar_pending columns: {}", columns.len());
    let expected = [
        ("id", "INTEGER", false, 1),
        ("source_generation", "INTEGER", true, 0),
        ("subject_key", "TEXT", true, 0),
        ("location_keys", "TEXT", true, 0),
        ("record", "TEXT", true, 0),
        ("queued_at", "INTEGER", true, 0),
    ];
    let expected: Vec<(String, String, bool, i64)> = expected
        .iter()
        .map(|(n, t, nn, pk)| ((*n).to_owned(), (*t).to_owned(), *nn, *pk))
        .collect();
    assert_eq!(columns, expected);

    let indexes: Vec<String> = conn
        .prepare("SELECT name FROM pragma_index_list('sidecar_pending') WHERE origin = 'c'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(indexes, vec!["idx_sidecar_pending_subject".to_owned()]);
    let index_columns: Vec<String> = conn
        .prepare("SELECT name FROM pragma_index_info('idx_sidecar_pending_subject')")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(index_columns, vec!["subject_key".to_owned()]);

    let foreign_keys: i64 = conn
        .query_row(
            "SELECT count(*) FROM pragma_foreign_key_list('sidecar_pending')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    eprintln!("sidecar_pending foreign keys: {foreign_keys}");
    assert_eq!(foreign_keys, 0, "a pending record must reference nothing");

    let sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'sidecar_pending'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(sql.contains("STRICT"), "{sql}");
    assert!(!sql.contains("AUTOINCREMENT"), "{sql}");
}

/// Each `NOT NULL` column refuses a row that omits it — five refusals, one per column, printed so a
/// loop that ran over nothing cannot pass.
#[test]
fn a_pending_row_needs_every_not_null_column() {
    let (_dir, conn) = fresh();
    let all = [
        ("source_generation", "1"),
        ("subject_key", "'lineage:ab'"),
        ("location_keys", "'[]'"),
        ("record", "'{}'"),
        ("queued_at", "1700000000"),
    ];
    let mut refused = 0_u32;
    for omitted in all.iter().map(|(name, _)| *name) {
        let kept: Vec<&(&str, &str)> = all.iter().filter(|(name, _)| *name != omitted).collect();
        let names: Vec<&str> = kept.iter().map(|(name, _)| *name).collect();
        let values: Vec<&str> = kept.iter().map(|(_, value)| *value).collect();
        let sql = format!(
            "INSERT INTO sidecar_pending ({}) VALUES ({})",
            names.join(", "),
            values.join(", ")
        );
        if conn.execute(&sql, []).is_err() {
            refused += 1;
        } else {
            eprintln!("admitted a row without {omitted}");
        }
    }
    eprintln!("rows refused for a missing NOT NULL column: {refused}");
    assert_eq!(refused, 5);

    let names: Vec<&str> = all.iter().map(|(name, _)| *name).collect();
    let values: Vec<&str> = all.iter().map(|(_, value)| *value).collect();
    conn.execute(
        &format!(
            "INSERT INTO sidecar_pending ({}) VALUES ({})",
            names.join(", "),
            values.join(", ")
        ),
        [],
    )
    .unwrap();
}

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::index::{open_connection, Index, IndexError};

#[test]
fn pragmas_are_the_ones_section_1_requires() {
    let dir = tempfile::tempdir().unwrap();
    let db = Index::db_path(dir.path());
    let conn = open_connection(&db).unwrap();

    let journal: String = conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(journal.to_ascii_lowercase(), "wal");

    let foreign_keys: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .unwrap();
    assert_eq!(foreign_keys, 1);

    let busy: i64 = conn
        .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
        .unwrap();
    assert_eq!(busy, 5000);

    let synchronous: i64 = conn
        .query_row("PRAGMA synchronous", [], |r| r.get(0))
        .unwrap();
    assert_eq!(synchronous, 1);
}

#[test]
fn a_second_connection_is_refused_because_there_is_one_writer() {
    let dir = tempfile::tempdir().unwrap();
    let db = Index::db_path(dir.path());
    let first = open_connection(&db).unwrap();

    match open_connection(&db) {
        Err(IndexError::AlreadyOpen { path }) => assert_eq!(path, db),
        Err(other) => panic!("expected AlreadyOpen, got {other:?}"),
        Ok(_) => panic!("a second connection was opened; there must be exactly one"),
    }

    drop(first);
    open_connection(&db).unwrap();
}

#[test]
fn db_path_and_siblings_live_under_the_data_dir() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(Index::db_path(dir.path()), dir.path().join("index.db"));
    assert_eq!(
        Index::sidecar_path(dir.path()),
        dir.path().join("index-sidecar.json")
    );
    assert_eq!(Index::backup_dir(dir.path()), dir.path().join("backups"));
}

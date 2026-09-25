#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! Opening the index: its pragmas, the one-writer rule, and the migration and backup on open.

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

use codotheca_core::index::migrate::SUPPORTED_SCHEMA_VERSION;

#[test]
fn open_migrates_a_fresh_directory_all_the_way_up() {
    let dir = tempfile::tempdir().unwrap();
    let index = Index::open_at(dir.path(), 1_000).unwrap();
    assert_eq!(index.schema_version().unwrap(), SUPPORTED_SCHEMA_VERSION);

    let mirror: String = index
        .conn()
        .query_row("SELECT v FROM app_meta WHERE k='schema_version'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(mirror, SUPPORTED_SCHEMA_VERSION.to_string());
}

#[test]
fn a_fresh_database_is_not_backed_up_because_there_is_nothing_to_lose() {
    let dir = tempfile::tempdir().unwrap();
    let _index = Index::open_at(dir.path(), 1_000).unwrap();
    let backups = Index::backup_dir(dir.path());
    assert!(
        !backups.exists() || std::fs::read_dir(&backups).unwrap().next().is_none(),
        "a backup of an empty database is noise in the data directory"
    );
}

#[test]
fn reopening_is_idempotent_and_writes_no_second_backup() {
    let dir = tempfile::tempdir().unwrap();
    drop(Index::open_at(dir.path(), 1_000).unwrap());
    let index = Index::open_at(dir.path(), 2_000).unwrap();
    assert_eq!(index.schema_version().unwrap(), SUPPORTED_SCHEMA_VERSION);
    let backups = Index::backup_dir(dir.path());
    assert!(!backups.exists() || std::fs::read_dir(&backups).unwrap().next().is_none());
}

#[test]
fn a_future_schema_refuses_at_the_open_door() {
    let dir = tempfile::tempdir().unwrap();
    {
        let conn = open_connection(&Index::db_path(dir.path())).unwrap();
        conn.execute_batch("PRAGMA user_version = 9999;").unwrap();
    }
    match Index::open_at(dir.path(), 1_000) {
        Err(IndexError::SchemaFromFuture { on_disk, supported }) => {
            assert_eq!(on_disk, 9999);
            assert_eq!(supported, SUPPORTED_SCHEMA_VERSION);
        }
        other => panic!("expected SchemaFromFuture, got {other:?}"),
    }
}

#[test]
fn a_mirror_that_disagrees_refuses_rather_than_guessing_which_is_right() {
    let dir = tempfile::tempdir().unwrap();
    drop(Index::open_at(dir.path(), 1_000).unwrap());
    {
        let conn = open_connection(&Index::db_path(dir.path())).unwrap();
        conn.execute("UPDATE app_meta SET v='2' WHERE k='schema_version'", [])
            .unwrap();
    }
    match Index::open_at(dir.path(), 2_000) {
        Err(IndexError::VersionMirrorMismatch { pragma, meta }) => {
            assert_eq!(pragma, SUPPORTED_SCHEMA_VERSION);
            assert_eq!(meta, 2);
        }
        other => panic!("expected VersionMirrorMismatch, got {other:?}"),
    }
}

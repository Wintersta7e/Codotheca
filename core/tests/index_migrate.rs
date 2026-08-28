#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::index::migrate::{
    apply_all, schema_version, Migration, MIGRATIONS, SUPPORTED_SCHEMA_VERSION,
};
use codotheca_core::index::{open_connection, Index, IndexError};
use codotheca_core::proto::txguard;

std::thread_local! {
    static BUSY_HANDLER_SAW_TX_GUARD: std::cell::Cell<Option<bool>> = const {
        std::cell::Cell::new(None)
    };
}

fn record_tx_guard(_attempts: i32) -> bool {
    BUSY_HANDLER_SAW_TX_GUARD.with(|saw| saw.set(Some(txguard::in_transaction())));
    false
}

fn scratch() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_connection(&Index::db_path(dir.path())).unwrap();
    (dir, conn)
}

const A: Migration = Migration {
    version: 1,
    name: "a",
    sql: "CREATE TABLE a (x INTEGER);",
};
const B: Migration = Migration {
    version: 2,
    name: "b",
    sql: "CREATE TABLE b (y INTEGER);",
};
/// Creates a table and then fails, so a rollback is observable as the table's absence.
const BROKEN: Migration = Migration {
    version: 3,
    name: "broken",
    sql: "CREATE TABLE c (z INTEGER); SELECT nonexistent_function(1);",
};

#[test]
fn applies_in_order_and_records_the_version() {
    let (_dir, mut conn) = scratch();
    assert_eq!(schema_version(&conn).unwrap(), 0);

    let reached = apply_all(&mut conn, &[A, B]).unwrap();
    assert_eq!(reached, 2);
    assert_eq!(schema_version(&conn).unwrap(), 2);

    for table in ["a", "b"] {
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "{table} was not created");
    }
}

#[test]
fn a_second_run_applies_nothing_and_is_not_an_error() {
    let (_dir, mut conn) = scratch();
    apply_all(&mut conn, &[A, B]).unwrap();
    assert_eq!(apply_all(&mut conn, &[A, B]).unwrap(), 2);
    assert_eq!(schema_version(&conn).unwrap(), 2);
}

#[test]
fn a_failing_migration_rolls_its_whole_transaction_back() {
    let (_dir, mut conn) = scratch();
    let err = apply_all(&mut conn, &[A, B, BROKEN]).unwrap_err();
    match err {
        IndexError::Sqlite(_) => {}
        other => panic!("expected the sqlite error to surface, got {other:?}"),
    }

    // Version stayed at the last good migration.
    assert_eq!(schema_version(&conn).unwrap(), 2);
    // And the table the broken migration created before failing is gone.
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='c'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0, "one transaction per migration — c must not survive");
}

#[test]
fn every_migration_activates_the_pipe_write_interlock() {
    let dir = tempfile::tempdir().unwrap();
    let db = Index::db_path(dir.path());
    let holder_db = db.clone();
    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        let mut conn = rusqlite::Connection::open(holder_db).unwrap();
        let _tx_guard = txguard::TxGuard::enter();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        locked_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        tx.rollback().unwrap();
    });
    locked_rx.recv().unwrap();

    let mut conn = rusqlite::Connection::open(&db).unwrap();
    BUSY_HANDLER_SAW_TX_GUARD.with(|saw| saw.set(None));
    conn.busy_handler(Some(record_tx_guard)).unwrap();

    let result = apply_all(&mut conn, &[A]);
    let saw_guard = BUSY_HANDLER_SAW_TX_GUARD.with(std::cell::Cell::get);
    release_tx.send(()).unwrap();
    holder.join().unwrap();

    assert!(result.is_err());
    assert_eq!(
        saw_guard,
        Some(true),
        "the SQLite busy callback ran inside a migration transaction without a TxGuard"
    );
}

#[test]
fn the_shipped_set_is_numbered_from_one_without_gaps() {
    for (i, m) in MIGRATIONS.iter().enumerate() {
        let expected = u32::try_from(i).unwrap() + 1;
        assert_eq!(m.version, expected, "{} is out of order", m.name);
        assert!(!m.sql.trim().is_empty(), "{} is empty", m.name);
    }
}

#[test]
fn the_supported_version_is_the_last_migration() {
    let last = MIGRATIONS.last().map_or(0, |m| m.version);
    assert_eq!(SUPPORTED_SCHEMA_VERSION, last);
}

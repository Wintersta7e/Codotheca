#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::index::migrate::{
    apply_all, guard_not_from_the_future, schema_version, Migration, MIGRATIONS,
    SUPPORTED_SCHEMA_VERSION,
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
    rebuilds_a_table: false,
};
const B: Migration = Migration {
    version: 2,
    name: "b",
    sql: "CREATE TABLE b (y INTEGER);",
    rebuilds_a_table: false,
};
/// Creates a table and then fails, so a rollback is observable as the table's absence.
const BROKEN: Migration = Migration {
    version: 3,
    name: "broken",
    sql: "CREATE TABLE c (z INTEGER); SELECT nonexistent_function(1);",
    rebuilds_a_table: false,
};

/// **The upgrade path, which no other test in this file walks.** Every migration test starts from
/// an empty database, where the `app_meta` mirror is absent and is written at the end — so the
/// mirror and the pragma agree by construction and the comparison between them can never fail.
///
/// A library that has been opened once carries a *stamped* mirror, and nothing updates it while
/// migrations run. Comparing it against the version migrations just **reached** therefore refuses
/// every upgrade, permanently. Measured on two real profiles on 2026-09-05, one Windows and one
/// WSL: `schema version mirror disagrees: user_version 9, app_meta 7`, then a crash loop.
#[test]
fn a_library_stamped_at_one_version_still_opens_after_a_new_migration_lands() {
    let dir = tempfile::tempdir().unwrap();
    // The real set: `app_meta` is created by `0001`, so the synthetic fixtures above have no such
    // table and the mirror cannot be exercised against them at all.
    {
        let index = codotheca_core::index::open_with_migrations(dir.path(), MIGRATIONS, 1).unwrap();
        assert_eq!(
            index.app_meta("schema_version").unwrap(),
            Some(SUPPORTED_SCHEMA_VERSION.to_string())
        );
    }
    let next = SUPPORTED_SCHEMA_VERSION + 1;
    let with_one_more: Vec<Migration> = MIGRATIONS
        .iter()
        .copied()
        .chain(std::iter::once(Migration {
            version: next,
            name: "one_more",
            sql: "CREATE TABLE one_more (x INTEGER);",
            rebuilds_a_table: false,
        }))
        .collect();

    let index = codotheca_core::index::open_with_migrations(dir.path(), &with_one_more, 2).unwrap();
    assert_eq!(schema_version(index.conn()).unwrap(), next);
    assert_eq!(
        index.app_meta("schema_version").unwrap(),
        Some(next.to_string()),
        "the mirror follows the migration that ran"
    );
}

/// The guard the change above must not weaken: a mirror disagreeing with the version **on disk**
/// still means something that is not this program wrote one of them.
#[test]
fn a_mirror_that_disagrees_with_the_version_on_disk_is_still_refused() {
    let dir = tempfile::tempdir().unwrap();
    {
        let index = codotheca_core::index::open_with_migrations(dir.path(), MIGRATIONS, 1).unwrap();
        index.set_app_meta("schema_version", "41").unwrap();
    }
    let err = codotheca_core::index::open_with_migrations(dir.path(), MIGRATIONS, 2).unwrap_err();
    assert!(
        matches!(
            err,
            IndexError::VersionMirrorMismatch {
                pragma: SUPPORTED_SCHEMA_VERSION,
                meta: 41
            }
        ),
        "{err:?}"
    );
}

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

#[test]
fn a_schema_from_the_future_refuses_and_names_both_numbers() {
    let (_dir, conn) = scratch();
    conn.execute_batch("PRAGMA user_version = 99;").unwrap();

    match guard_not_from_the_future(&conn, 5) {
        Err(IndexError::SchemaFromFuture { on_disk, supported }) => {
            assert_eq!(on_disk, 99);
            assert_eq!(supported, 5);
        }
        other => panic!("expected SchemaFromFuture, got {other:?}"),
    }
}

#[test]
fn the_current_and_older_versions_are_both_fine() {
    let (_dir, conn) = scratch();
    guard_not_from_the_future(&conn, 5).unwrap(); // 0: a fresh database
    conn.execute_batch("PRAGMA user_version = 5;").unwrap();
    guard_not_from_the_future(&conn, 5).unwrap(); // equal: current
    conn.execute_batch("PRAGMA user_version = 3;").unwrap();
    guard_not_from_the_future(&conn, 5).unwrap(); // older: migrate it
}

#[test]
fn apply_all_refuses_rather_than_writing_to_a_future_database() {
    let (_dir, mut conn) = scratch();
    conn.execute_batch("PRAGMA user_version = 99;").unwrap();
    match apply_all(&mut conn, &[A, B]) {
        Err(IndexError::SchemaFromFuture { on_disk, .. }) => assert_eq!(on_disk, 99),
        other => panic!("expected SchemaFromFuture, got {other:?}"),
    }
    // Nothing was created; refusing to open means refusing to write.
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0);
}

use codotheca_core::index::backup::{backup_before_migrating, restore_over, vacuum_into};

#[test]
fn a_file_copy_loses_a_committed_transaction_and_vacuum_into_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let db = Index::db_path(dir.path());
    let conn = open_connection(&db).unwrap();
    conn.execute_batch("CREATE TABLE t (v TEXT);").unwrap();
    conn.execute_batch("INSERT INTO t (v) VALUES ('committed');")
        .unwrap();

    // The row is committed but the WAL has not been checkpointed.
    let wal = dir.path().join("index.db-wal");
    assert!(wal.exists(), "expected a -wal file to exist in WAL mode");

    let naive = dir.path().join("naive-copy.db");
    std::fs::copy(&db, &naive).unwrap();

    let proper = dir.path().join("proper-copy.db");
    vacuum_into(&conn, &proper).unwrap();
    drop(conn);

    let naive_conn = rusqlite::Connection::open(&naive).unwrap();
    let naive_rows: i64 = naive_conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='t'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        naive_rows, 0,
        "the naive copy must be shown to lose the committed transaction — \
         this is why §1.12 forbids it"
    );

    let proper_conn = rusqlite::Connection::open(&proper).unwrap();
    let v: String = proper_conn
        .query_row("SELECT v FROM t", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, "committed");
}

#[test]
fn backup_before_migrating_names_the_version_it_came_from() {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_connection(&Index::db_path(dir.path())).unwrap();
    conn.execute_batch("CREATE TABLE t (v TEXT); PRAGMA user_version = 3;")
        .unwrap();

    let path = backup_before_migrating(&conn, dir.path(), 3, 1_787_126_520).unwrap();
    assert!(path.starts_with(Index::backup_dir(dir.path())));
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    assert_eq!(name, "index-pre-3-1787126520.db");
    assert!(path.exists());
}

#[test]
fn restore_over_replaces_the_database_and_clears_the_stale_wal() {
    let dir = tempfile::tempdir().unwrap();
    let db = Index::db_path(dir.path());
    let conn = open_connection(&db).unwrap();
    conn.execute_batch("CREATE TABLE good (v TEXT);").unwrap();
    let backup = backup_before_migrating(&conn, dir.path(), 1, 1).unwrap();
    conn.execute_batch("CREATE TABLE bad (v TEXT);").unwrap();
    drop(conn);

    restore_over(&backup, &db).unwrap();
    assert!(!dir.path().join("index.db-wal").exists());
    assert!(!dir.path().join("index.db-shm").exists());

    let conn = open_connection(&db).unwrap();
    let bad: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='bad'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        bad, 0,
        "the failed migration's table must not survive a restore"
    );
}

#[test]
fn a_failed_migration_restores_the_backup_and_names_what_it_restored_to() {
    let dir = tempfile::tempdir().unwrap();
    // A database that is genuinely at schema 1, with a row worth keeping.
    {
        let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
        apply_all(&mut conn, &MIGRATIONS[..1]).unwrap();
        conn.execute(
            "INSERT INTO project (name, seed_basename, created_at, updated_at)
             VALUES ('kept', 'kept', 1, 1)",
            [],
        )
        .unwrap();
    }

    // A migration set whose next step fails.
    let poisoned: Vec<Migration> = MIGRATIONS[..1]
        .iter()
        .copied()
        .chain(std::iter::once(Migration {
            version: 2,
            name: "poisoned",
            sql: "CREATE TABLE t (x INTEGER); SELECT nonexistent_function(1);",
            rebuilds_a_table: false,
        }))
        .collect();

    let err =
        codotheca_core::index::open_with_migrations(dir.path(), &poisoned, 4_242).unwrap_err();
    match err {
        IndexError::MigrationFailed {
            version,
            name,
            restored_to,
            restored_at,
            ..
        } => {
            assert_eq!(version, 2);
            assert_eq!(name, "poisoned");
            assert_eq!(restored_to, 1);
            assert_eq!(restored_at, 4_242);
        }
        other => panic!("expected MigrationFailed, got {other:?}"),
    }

    // The backup was restored: the row survives and the failed step's table is absent.
    let conn = open_connection(&Index::db_path(dir.path())).unwrap();
    assert_eq!(schema_version(&conn).unwrap(), 1);
    let kept: i64 = conn
        .query_row("SELECT count(*) FROM project WHERE name='kept'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(kept, 1);
    let leaked: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='t'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(leaked, 0);
}

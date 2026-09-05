//! `0009` — the one `project` rebuild phase 2 performs, and the runner that makes it possible.
//!
//! R59: **a table rebuild is a property of the migration, not of its SQL.** The file carries no
//! pragma; the runner takes the connection out of its transaction, disables foreign keys, reads
//! the value back to prove it took, runs the file, evaluates `foreign_key_check` in Rust, and
//! restores the prior value on both the success and the error path.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::index::migrate::{
    apply_all, guard_contiguous, schema_version, Migration, MIGRATIONS, SUPPORTED_SCHEMA_VERSION,
};
use codotheca_core::index::{open_connection, Index, IndexError};

/// A real database opened the way production opens one — so `foreign_keys` starts **ON**, which
/// is the state R59's premise turns on.
fn scratch() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_connection(&Index::db_path(dir.path())).unwrap();
    (dir, conn)
}

fn foreign_keys(conn: &rusqlite::Connection) -> i64 {
    conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .unwrap()
}

fn table_exists(conn: &rusqlite::Connection, table: &str) -> bool {
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |r| r.get(0),
        )
        .unwrap();
    n == 1
}

/// An ordinary migration: no rebuild, so the runner leaves the pragma alone.
const PLAIN: Migration = Migration {
    version: 1,
    name: "plain",
    sql: "CREATE TABLE plain (x INTEGER);",
    rebuilds_a_table: false,
};

/// Reads the connection's `foreign_keys` setting **from inside the migration transaction** and
/// stores it, so the assertion is about what the file actually ran under rather than about what
/// the runner claims it set.
const READS_THE_PRAGMA: Migration = Migration {
    version: 2,
    name: "reads_the_pragma",
    sql: "CREATE TABLE fk_probe (v INTEGER NOT NULL);
          INSERT INTO fk_probe (v) SELECT foreign_keys FROM pragma_foreign_keys;",
    rebuilds_a_table: true,
};

/// Leaves a dangling reference behind. It can only be *written* with enforcement off, which is
/// the point: without the post-file `foreign_key_check` the runner would commit a broken database.
const LEAVES_A_DANGLING_REFERENCE: Migration = Migration {
    version: 2,
    name: "leaves_a_dangling_reference",
    sql: "CREATE TABLE fk_parent (id INTEGER PRIMARY KEY);
          CREATE TABLE fk_child (
            id        INTEGER PRIMARY KEY,
            parent_id INTEGER NOT NULL REFERENCES fk_parent(id)
          );
          INSERT INTO fk_child (id, parent_id) VALUES (1, 999);",
    rebuilds_a_table: true,
};

/// **This test asserts the trap, so it passes before and after the fix.** It is the evidence that
/// R59's machinery is necessary at all: `apply_all` wraps every file in a transaction, and
/// `PRAGMA foreign_keys` is a documented no-op inside one — so a `PRAGMA foreign_keys=OFF` written
/// into `0009.sql` would silently do nothing and `DROP TABLE project` would fire six
/// `ON DELETE CASCADE` children with enforcement still on.
#[test]
fn the_pragma_is_a_no_op_inside_a_transaction() {
    let (_dir, mut conn) = scratch();
    assert_eq!(
        foreign_keys(&conn),
        1,
        "production opens with enforcement on"
    );

    let tx = conn.transaction().unwrap();
    tx.execute_batch("PRAGMA foreign_keys=OFF;").unwrap();
    let inside: i64 = tx
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .unwrap();
    tx.rollback().unwrap();

    assert_eq!(
        inside, 1,
        "the pragma is a no-op inside a transaction — this is why the toggle lives in the runner"
    );
}

#[test]
fn a_rebuild_migration_runs_with_foreign_keys_off() {
    let (_dir, mut conn) = scratch();
    assert_eq!(foreign_keys(&conn), 1);

    apply_all(&mut conn, &[PLAIN, READS_THE_PRAGMA]).unwrap();

    let seen: i64 = conn
        .query_row("SELECT v FROM fk_probe", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        seen, 0,
        "a rebuilding migration must run with foreign_keys OFF"
    );
    assert_eq!(
        foreign_keys(&conn),
        1,
        "the prior value is restored after the file runs"
    );
}

#[test]
fn a_plain_migration_runs_with_the_pragma_untouched() {
    // The toggle is scoped to the migration that asked for it. A build that disabled enforcement
    // for every file would make this pass with `0` and would widen R59's window to the whole set.
    let (_dir, mut conn) = scratch();
    let probe = Migration {
        rebuilds_a_table: false,
        ..READS_THE_PRAGMA
    };
    apply_all(&mut conn, &[PLAIN, probe]).unwrap();

    let seen: i64 = conn
        .query_row("SELECT v FROM fk_probe", [], |r| r.get(0))
        .unwrap();
    assert_eq!(seen, 1, "a non-rebuilding migration keeps enforcement on");
}

/// The read-back is the whole point, so it needs a case where the set genuinely does not take.
///
/// An open transaction is exactly that case, and it is the same one R59 is about: inside one the
/// pragma is a documented no-op. The runner must **apply nothing** rather than run a
/// create-copy-drop-rename with enforcement still on.
#[test]
fn a_rebuild_refuses_when_the_pragma_would_not_take() {
    let (_dir, mut conn) = scratch();
    apply_all(&mut conn, &[PLAIN]).unwrap();
    conn.execute_batch("BEGIN;").unwrap();

    let err = apply_all(&mut conn, &[PLAIN, READS_THE_PRAGMA]).unwrap_err();
    match err {
        IndexError::ForeignKeysNotDisabled { reported } => assert_eq!(reported, 1),
        other => panic!("expected ForeignKeysNotDisabled, got {other:?}"),
    }
    conn.execute_batch("ROLLBACK;").unwrap();

    assert!(
        !table_exists(&conn, "fk_probe"),
        "a rebuild whose pragma did not take applies nothing"
    );
    assert_eq!(schema_version(&conn).unwrap(), 1);
}

#[test]
fn a_violating_rebuild_rolls_back_and_names_the_count() {
    let (_dir, mut conn) = scratch();
    let err = apply_all(&mut conn, &[PLAIN, LEAVES_A_DANGLING_REFERENCE]).unwrap_err();

    match err {
        IndexError::ForeignKeyViolations { count } => {
            assert_eq!(count, 1, "one dangling reference, counted in Rust");
        }
        other => panic!("expected ForeignKeyViolations, got {other:?}"),
    }

    assert_eq!(
        schema_version(&conn).unwrap(),
        1,
        "the failed rebuild's version was never stamped"
    );
    assert!(
        !table_exists(&conn, "fk_child"),
        "dropping the transaction rolls the whole file back"
    );
    assert_eq!(
        foreign_keys(&conn),
        1,
        "the prior value is restored on the error path too"
    );
}

#[test]
fn a_holed_chain_is_refused_and_names_the_missing_version() {
    let holed = [
        PLAIN,
        Migration {
            version: 3,
            ..PLAIN
        },
    ];
    match guard_contiguous(&holed) {
        Err(IndexError::MigrationChainBroken { expected, found }) => {
            assert_eq!((expected, found), (2, 3));
        }
        other => panic!("expected MigrationChainBroken, got {other:?}"),
    }

    let repeated = [
        PLAIN,
        Migration {
            version: 2,
            ..PLAIN
        },
        Migration {
            version: 2,
            ..PLAIN
        },
    ];
    match guard_contiguous(&repeated) {
        Err(IndexError::MigrationChainBroken { expected, found }) => {
            assert_eq!(
                (expected, found),
                (3, 2),
                "a repeat is a hole one step later"
            );
        }
        other => panic!("expected MigrationChainBroken on a repeat, got {other:?}"),
    }

    // A guard that validated an empty list is a failing guard: `apply_all(&[])` returns Ok(0)
    // today having asserted nothing at all.
    match guard_contiguous(&[]) {
        Err(IndexError::MigrationChainEmpty) => {}
        other => panic!("expected MigrationChainEmpty, got {other:?}"),
    }

    let checked = guard_contiguous(MIGRATIONS).unwrap();
    eprintln!("guard_contiguous checked {checked} registered migrations");
    assert!(checked > 0, "a guard that checked nothing cannot pass");
    assert_eq!(
        checked,
        u32::try_from(MIGRATIONS.len()).unwrap(),
        "the count returned is the count of registered migrations"
    );
    assert_eq!(
        checked, SUPPORTED_SCHEMA_VERSION,
        "the chain ends at the version this build claims to understand"
    );
}

#[test]
fn a_holed_chain_refuses_rather_than_skipping_the_missing_file() {
    // R68's defect: `apply_all` skips any migration with `version <= current` and checks no
    // contiguity, so a database that reached the far side of a hole skips the missing file
    // **forever**, silently, on the user's machine.
    let (_dir, mut conn) = scratch();
    let holed = [
        PLAIN,
        Migration {
            version: 3,
            name: "past_the_hole",
            sql: "CREATE TABLE past_the_hole (x INTEGER);",
            rebuilds_a_table: false,
        },
    ];
    match apply_all(&mut conn, &holed) {
        Err(IndexError::MigrationChainBroken { expected, found }) => {
            assert_eq!((expected, found), (2, 3));
        }
        other => panic!("expected apply_all to refuse a holed chain, got {other:?}"),
    }
    assert!(
        !table_exists(&conn, "plain"),
        "a refused chain applies nothing at all"
    );
}

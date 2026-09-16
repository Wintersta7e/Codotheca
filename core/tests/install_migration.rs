#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! Migration 0010's stored vocabulary and schema delta against real migrated databases.

use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::install::InstallRunState;
use rusqlite::{params, Connection};

const INSTALL_SCHEMA_VERSION: u32 = 10;

fn migrated_through(version: u32) -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    let migrations: Vec<_> = MIGRATIONS
        .iter()
        .copied()
        .take_while(|migration| migration.version <= version)
        .collect();
    apply_all(&mut conn, &migrations).unwrap();
    (dir, conn)
}

fn columns(conn: &Connection, table: &str) -> Vec<String> {
    let mut statement = conn
        .prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
        .unwrap();
    statement
        .query_map([table], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn insert_dependencies(conn: &Connection) -> (i64, i64) {
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES ('project', 'project', 1, 1)",
        [],
    )
    .unwrap();
    let project_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO scan_root
           (kind, distro, path_bytes, path_key, path_display, added_by, added_at)
         VALUES ('linux', '', ?1, ?1, 'tmp_path', 'user', 1)",
        [b"tmp_path".as_slice()],
    )
    .unwrap();
    (project_id, conn.last_insert_rowid())
}

#[test]
fn install_migration_matches_the_rust_states_and_location_delta() {
    let (_dir, conn) = migrated_through(INSTALL_SCHEMA_VERSION);
    let user_version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        user_version, INSTALL_SCHEMA_VERSION,
        "migration 0010 must set PRAGMA user_version to 10"
    );

    let (_before_dir, before) = migrated_through(INSTALL_SCHEMA_VERSION - 1);
    let before_columns = columns(&before, "location");
    let after_columns = columns(&conn, "location");
    let added: Vec<_> = after_columns
        .iter()
        .filter(|column| !before_columns.contains(column))
        .cloned()
        .collect();
    let removed: Vec<_> = before_columns
        .iter()
        .filter(|column| !after_columns.contains(column))
        .cloned()
        .collect();
    assert_eq!(added, ["removed_at"]);
    assert!(
        removed.is_empty(),
        "migration 0010 removed columns: {removed:?}"
    );

    let (project_id, root_id) = insert_dependencies(&conn);
    for state in InstallRunState::ALL {
        conn.execute(
            "INSERT INTO install_run
               (project_id, root_id, staging_bytes, destination_bytes, state, started_at, ended_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, NULL)",
            params![
                project_id,
                root_id,
                b"staging".as_slice(),
                b"destination".as_slice(),
                state.as_str()
            ],
        )
        .unwrap_or_else(|error| {
            panic!(
                "InstallRunState::{state:?} slug {:?} was refused by install_run.state: {error}",
                state.as_str()
            )
        });
    }

    let bogus = conn.execute(
        "INSERT INTO install_run
           (project_id, root_id, staging_bytes, destination_bytes, state, started_at, ended_at)
         VALUES (?1, ?2, ?3, ?4, 'bogus', 1, NULL)",
        params![
            project_id,
            root_id,
            b"staging".as_slice(),
            b"destination".as_slice()
        ],
    );
    assert!(bogus.is_err(), "install_run.state accepted a bogus slug");
}

/// `install_run`'s two foreign keys, exercised rather than declared.
///
/// `every_foreign_key_into_project_survives_the_rebuild` stops at the one migration that rebuilds
/// a table — correctly, because a key `0010` adds afterwards is not a rebuild defect — so **no
/// other test in the tree reaches these two**. A declared constraint nothing exercises is the
/// shape this project keeps paying for: it compiles, it reads as protection, and it is only ever
/// true by assumption.
///
/// The pragma is read back rather than assumed: `open_connection` sets `foreign_keys=ON`
/// (`core/src/index/mod.rs`), and with it off both inserts below would succeed and this test would
/// pass while asserting nothing.
#[test]
fn install_run_refuses_a_row_whose_project_or_root_does_not_exist() {
    let (_dir, conn) = migrated_through(INSTALL_SCHEMA_VERSION);
    let enforced: bool = conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert!(
        enforced,
        "foreign_keys is off, so both inserts below would succeed and prove nothing"
    );

    let (project_id, root_id) = insert_dependencies(&conn);
    let insert = |project: i64, root: i64| {
        conn.execute(
            "INSERT INTO install_run
               (project_id, root_id, staging_bytes, destination_bytes, state, started_at, ended_at)
             VALUES (?1, ?2, ?3, ?4, 'running', 1, NULL)",
            params![
                project,
                root,
                b"staging".as_slice(),
                b"destination".as_slice()
            ],
        )
    };

    assert!(
        insert(project_id + 9_000, root_id).is_err(),
        "install_run accepted a project_id no project row has"
    );
    assert!(
        insert(project_id, root_id + 9_000).is_err(),
        "install_run accepted a root_id no scan_root row has"
    );
    assert!(
        insert(project_id, root_id).is_ok(),
        "the valid pair must still insert, or the two refusals above prove only that the \
         statement is broken"
    );
}

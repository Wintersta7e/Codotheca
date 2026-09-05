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

/// A database at exactly `version`, through the shipped migration set.
fn migrated_to(version: usize) -> (tempfile::TempDir, rusqlite::Connection) {
    let (dir, mut conn) = scratch();
    apply_all(&mut conn, &MIGRATIONS[..version]).unwrap();
    (dir, conn)
}

fn columns(conn: &rusqlite::Connection, table: &str) -> Vec<String> {
    let mut st = conn
        .prepare(&format!("PRAGMA table_info(\"{table}\")"))
        .unwrap();
    let rows = st.query_map([], |r| r.get::<_, String>(1)).unwrap();
    rows.map(Result::unwrap).collect()
}

fn tables(conn: &rusqlite::Connection) -> Vec<String> {
    let mut st = conn
        .prepare(
            "SELECT name FROM sqlite_schema
              WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .unwrap();
    let rows = st.query_map([], |r| r.get::<_, String>(0)).unwrap();
    rows.map(Result::unwrap).collect()
}

/// Every index on `table`, as `(name, the stored CREATE statement)`, ordered by name.
///
/// **The definition, not the name.** `idx_project_shelf_order` is partial and
/// `idx_project_reference_order` is composite; a rebuild that recreated either one plainly would
/// pass a name check and silently change the shelf's query plan and its row set.
fn index_definitions(conn: &rusqlite::Connection, table: &str) -> Vec<(String, String)> {
    let mut st = conn
        .prepare(
            "SELECT name, sql FROM sqlite_schema
              WHERE type='index' AND tbl_name=?1 AND sql IS NOT NULL ORDER BY name",
        )
        .unwrap();
    let rows = st
        .query_map([table], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .unwrap();
    rows.map(Result::unwrap).collect()
}

/// R80: every foreign key in the database that points at `project`, as one comparable set.
///
/// **A per-column diff cannot see a lost cascade.** `delete_account` reads `PRAGMA foreign_keys`
/// back and refuses if it is off, and `0009` is the one migration that turns it off — over a
/// table `project_account` cascades into. A rebuild that dropped that cascade would leave the
/// guard passing and protecting nothing, because the guard reads a pragma and cannot see a
/// missing foreign key.
fn foreign_keys_into_project(conn: &rusqlite::Connection) -> Vec<String> {
    let mut out = Vec::new();
    for table in tables(conn) {
        let mut st = conn
            .prepare(&format!("PRAGMA foreign_key_list(\"{table}\")"))
            .unwrap();
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                ))
            })
            .unwrap();
        for row in rows {
            let (target, from, to, on_update, on_delete) = row.unwrap();
            if target == "project" {
                let to = to.unwrap_or_else(|| "id".to_owned());
                out.push(format!(
                    "{table}.{from} -> project.{to} ON UPDATE {on_update} ON DELETE {on_delete}"
                ));
            }
        }
    }
    out.sort();
    out
}

fn rows_as_text(conn: &rusqlite::Connection, table: &str, cols: &[String]) -> Vec<Vec<String>> {
    let list = cols
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let mut st = conn
        .prepare(&format!("SELECT {list} FROM \"{table}\" ORDER BY id"))
        .unwrap();
    let mut rows = st.query([]).unwrap();
    let mut out = Vec::new();
    while let Some(row) = rows.next().unwrap() {
        let mut one = Vec::new();
        for i in 0..cols.len() {
            let value: rusqlite::types::Value = row.get(i).unwrap();
            one.push(format!("{value:?}"));
        }
        out.push(one);
    }
    out
}

fn count(conn: &rusqlite::Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT count(*) FROM \"{table}\""), [], |r| {
        r.get(0)
    })
    .unwrap()
}

/// The seven tables that cascade off `project`, each seeded so a cascade firing during the
/// rebuild is visible as a count of zero rather than as an empty fixture.
const CASCADING_CHILDREN: &[&str] = &[
    "art_scene",
    "collection_member",
    "fts_commits",
    "peek_cache",
    "project_account",
    "project_committer",
    "project_job_state",
];

/// Three projects, one account, and one row in every cascading child of `project`.
fn seed_a_library(conn: &rusqlite::Connection) {
    for name in ["alpha", "beta", "gamma"] {
        conn.execute(
            "INSERT INTO project (lineage_key, remote_key, name, seed_basename, description,
                                  description_source, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3, 'a description', 'readme', 100, 100)",
            rusqlite::params![
                format!("lin-{name}"),
                format!("forge.example/acme/{name}"),
                name
            ],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO account (provider, host, login, auth_kind, scope_tier, granted_scopes,
                              token_ref, connected_at)
         VALUES ('github', 'github.com', 'someone', 'pat', 'public', '[]', 'ref', 10)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO collection (name, kind) VALUES ('c', 'manual')",
        [],
    )
    .unwrap();

    for (sql, table) in [
        (
            "INSERT INTO art_scene (project_id, scene_hash, scene_json, schema_version, state)
             VALUES (1, 'hash', '{}', 1, 'ready')",
            "art_scene",
        ),
        (
            "INSERT INTO collection_member (collection_id, project_id) VALUES (1, 1)",
            "collection_member",
        ),
        (
            "INSERT INTO fts_commits (project_id, subjects) VALUES (1, 'a subject')",
            "fts_commits",
        ),
        (
            "INSERT INTO peek_cache (project_id, computed_at) VALUES (1, 9)",
            "peek_cache",
        ),
        (
            "INSERT INTO project_account (project_id, account_id, affiliation, can_push,
                                          observed_at)
             VALUES (1, 1, 'owner', 1, 11)",
            "project_account",
        ),
        (
            "INSERT INTO project_committer (project_id, email, commits)
             VALUES (1, 'someone@example.invalid', 3)",
            "project_committer",
        ),
        (
            "INSERT INTO project_job_state (project_id, job, state, at)
             VALUES (1, 'j1', 'ok', 12)",
            "project_job_state",
        ),
    ] {
        conn.execute(sql, [])
            .unwrap_or_else(|e| panic!("{table}: {e}"));
    }
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

// ---------------------------------------------------------------------------------------------
// Task 2 — `0009`, the one `project` rebuild.
// ---------------------------------------------------------------------------------------------

/// The four columns §22.11 and §25.7 add. **Four, not five**: the fifth *change* is the
/// `description_source` CHECK widening, which adds no column and is asserted separately.
const ADDED_COLUMNS: &[&str] = &[
    "provider",
    "provider_repo_id",
    "readme_remote_at",
    "remote_link_basis",
];

#[test]
fn the_column_set_is_the_old_set_plus_exactly_four() {
    let (_dir, mut conn) = migrated_to(8);
    let before = columns(&conn, "project");
    apply_all(&mut conn, MIGRATIONS).unwrap();
    let after = columns(&conn, "project");

    // As one set difference over the whole list. Never four separate equalities: a per-column
    // test passes while a fifth column silently vanishes.
    let mut added: Vec<&String> = after.iter().filter(|c| !before.contains(c)).collect();
    let removed: Vec<&String> = before.iter().filter(|c| !after.contains(c)).collect();
    added.sort();

    assert_eq!(
        removed,
        Vec::<&String>::new(),
        "the rebuild dropped a column\nbefore: {before:?}\nafter: {after:?}"
    );
    assert_eq!(
        added.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
        ADDED_COLUMNS,
        "the added set is not exactly the four\nbefore: {before:?}\nafter: {after:?}"
    );
    assert_eq!(before.len(), 50, "the pre-rebuild column count is 50");
    assert_eq!(after.len(), 54);
}

#[test]
fn every_row_and_every_index_survives_the_rebuild() {
    let (_dir, mut conn) = migrated_to(8);
    seed_a_library(&conn);

    let before_columns = columns(&conn, "project");
    let before_rows = rows_as_text(&conn, "project", &before_columns);
    let before_indexes = index_definitions(&conn, "project");
    let before_children: Vec<i64> = CASCADING_CHILDREN.iter().map(|t| count(&conn, t)).collect();
    assert!(
        before_children.iter().all(|n| *n > 0),
        "the fixture must seed every cascading child, or a count of zero afterwards proves \
         nothing: {before_children:?}"
    );

    apply_all(&mut conn, MIGRATIONS).unwrap();

    assert_eq!(
        rows_as_text(&conn, "project", &before_columns),
        before_rows,
        "the copied rows are not column-for-column what they were"
    );
    let after_children: Vec<i64> = CASCADING_CHILDREN.iter().map(|t| count(&conn, t)).collect();
    assert_eq!(
        after_children, before_children,
        "a cascade fired during the rebuild: {CASCADING_CHILDREN:?}"
    );

    // The definitions, byte for byte, plus exactly the one new index.
    let after_indexes = index_definitions(&conn, "project");
    let kept: Vec<(String, String)> = after_indexes
        .iter()
        .filter(|(name, _)| name != "idx_project_provider_repo")
        .cloned()
        .collect();
    assert_eq!(kept, before_indexes, "an index definition changed");
    assert_eq!(after_indexes.len(), before_indexes.len() + 1);
    assert!(after_indexes
        .iter()
        .any(|(name, _)| name == "idx_project_provider_repo"));
}

/// **R80.** The column diff above cannot see this, and neither can the contiguity guard in
/// `migrate.rs`, which checks version numbers and not referential integrity.
#[test]
fn every_foreign_key_into_project_survives_the_rebuild() {
    let (_dir, mut conn) = migrated_to(8);
    seed_a_library(&conn);

    let before = foreign_keys_into_project(&conn);
    assert!(
        before.iter().any(|fk| fk.starts_with("project_account.")),
        "the pre-rebuild set must contain the account link, or the comparison is vacuous: \
         {before:?}"
    );
    let cascading = before
        .iter()
        .filter(|fk| fk.ends_with("ON DELETE CASCADE"))
        .count();
    assert_eq!(cascading, 7, "seven cascading children: {before:?}");

    apply_all(&mut conn, MIGRATIONS).unwrap();

    assert_eq!(
        foreign_keys_into_project(&conn),
        before,
        "the foreign-key set into project changed across the rebuild"
    );
    assert_eq!(
        count(&conn, "project_account"),
        1,
        "project_account's rows must survive — a cascade deletes silently and a count of zero \
         looks like an empty fixture"
    );
}

#[test]
fn the_autoincrement_high_water_mark_survives() {
    let (_dir, mut conn) = migrated_to(8);
    seed_a_library(&conn);
    let before: i64 = conn
        .query_row(
            "SELECT seq FROM sqlite_sequence WHERE name='project'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    // §1.5: "a tombstoned rowid can never be reused", and that guarantee lives entirely in the
    // high-water mark. The copy re-seeds it from the ids actually inserted.
    conn.execute("DELETE FROM project WHERE id = ?1", [before])
        .unwrap();

    apply_all(&mut conn, MIGRATIONS).unwrap();

    let after: i64 = conn
        .query_row(
            "SELECT seq FROM sqlite_sequence WHERE name='project'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(after, before, "the mark was lowered to the new max(id)");
}

#[test]
fn the_rebuilt_table_is_strict_and_keeps_its_self_references() {
    let (_dir, conn) = migrated_to(9);
    let sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name='project'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(sql.contains("STRICT"), "the rebuilt table lost STRICT");
    assert_eq!(
        sql.matches("REFERENCES project(id)").count(),
        2,
        "parent_project_id and merged_into both point back at project"
    );
    assert_eq!(
        sql.matches("project_new").count(),
        0,
        "the rename left the scratch name in the stored schema"
    );
}

/// §22.11 rules against a constraint here in the sentence that describes the case one would
/// forbid: two projects on one forge repository share one facts row and one clock, "what keeps
/// §22.5's non-unique case from needing two of either". Under a UNIQUE index §22.5's second row
/// becomes a failed transaction where the section requires ambiguity.
#[test]
fn the_provider_repo_index_is_not_unique() {
    let (_dir, conn) = migrated_to(9);
    let mut st = conn.prepare("PRAGMA index_list(project)").unwrap();
    let rows = st
        .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))
        .unwrap();
    let unique = rows
        .map(Result::unwrap)
        .find(|(name, _)| name == "idx_project_provider_repo")
        .map(|(_, unique)| unique);
    assert_eq!(
        unique,
        Some(0),
        "idx_project_provider_repo must not be UNIQUE"
    );
    drop(st);

    // And the behaviour the index must not forbid, asserted rather than described.
    conn.execute(
        "INSERT INTO project (name, seed_basename, provider, provider_repo_id, created_at,
                              updated_at)
         VALUES ('one', 'one', 'github', '42', 1, 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO project (name, seed_basename, provider, provider_repo_id, created_at,
                              updated_at)
         VALUES ('two', 'two', 'github', '42', 1, 1)",
        [],
    )
    .expect("§22.5 needs two live projects on one pair to be representable");
}

/// **AC-P2-25-25, the structural half.** The stable id lives on `project`; nothing anywhere keys
/// identity on `remote_key`, which is a value a rename moves.
#[test]
fn no_table_takes_remote_key_as_a_primary_or_unique_key() {
    let (_dir, conn) = migrated_to(9);
    let mut scanned = 0_usize;
    let mut carrying = 0_usize;
    for table in tables(&conn) {
        scanned += 1;
        let mut info = conn
            .prepare(&format!("PRAGMA table_info(\"{table}\")"))
            .unwrap();
        let pk_columns: Vec<String> = info
            .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(5)?)))
            .unwrap()
            .map(Result::unwrap)
            .filter(|(_, pk)| *pk > 0)
            .map(|(name, _)| name)
            .collect();
        if columns(&conn, &table).iter().any(|c| c == "remote_key") {
            carrying += 1;
        }
        assert!(
            !pk_columns.iter().any(|c| c == "remote_key"),
            "{table} takes remote_key as a primary key"
        );

        let mut list = conn
            .prepare(&format!("PRAGMA index_list(\"{table}\")"))
            .unwrap();
        let unique_indexes: Vec<String> = list
            .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .filter(|(_, unique)| *unique == 1)
            .map(|(name, _)| name)
            .collect();
        for index in unique_indexes {
            let mut members = conn
                .prepare(&format!("PRAGMA index_info(\"{index}\")"))
                .unwrap();
            let names: Vec<Option<String>> = members
                .query_map([], |r| r.get::<_, Option<String>>(2))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            assert!(
                !names.iter().any(|c| c.as_deref() == Some("remote_key")),
                "{table}.{index} is a UNIQUE index over remote_key"
            );
        }
    }
    eprintln!("scanned {scanned} tables, {carrying} of them carrying a remote_key column");
    assert!(scanned > 0, "a gate whose passing run scans nothing fails");
    assert!(
        carrying > 0,
        "no table carries remote_key at all — the scan proves nothing"
    );
}

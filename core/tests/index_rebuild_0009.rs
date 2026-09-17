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

    // The chain runs **through the rebuild and stops there**. A later migration that legitimately
    // adds a foreign key into `project` — `0010`'s `install_run` does — is not a rebuild defect,
    // and letting it into this comparison would fail the assertion for the one reason it is not
    // about. Derived from `rebuilds_a_table` rather than sliced at a position, so inserting or
    // renumbering a migration cannot silently move what this test runs.
    assert_eq!(
        MIGRATIONS.iter().filter(|m| m.rebuilds_a_table).count(),
        1,
        "R80's subject is the one rebuild; with two, the search below silently picks the first"
    );
    let rebuild_at = MIGRATIONS
        .iter()
        .position(|m| m.rebuilds_a_table)
        .expect("phase 2 performs exactly one table rebuild");
    apply_all(&mut conn, &MIGRATIONS[..=rebuild_at]).unwrap();

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

// ---------------------------------------------------------------------------------------------
// Task 3 — §25.7's three remote-fact tables.
// ---------------------------------------------------------------------------------------------

/// §25.7's column lists, whole. The key assertion below passes while `permitted`, `ci_etag`,
/// `ci_observed_at`, `good_first_issues`, `open_prs_from_user` or `fork_parent_remote_key` is
/// missing — and every one of those is a column §21 or §25 writes without owning the DDL, so a
/// silently absent column surfaces three waves later as a runtime `no such column`.
const FACT_TABLE_COLUMNS: &[(&str, &[&str])] = &[
    (
        "remote_repo",
        &[
            "provider",
            "provider_repo_id",
            "visibility",
            "description",
            "fork_parent_remote_key",
            "stars",
            "open_issues",
            "good_first_issues",
            "open_prs",
            "open_prs_from_user",
            "permitted",
            "observed_at",
            "etag",
            "ci_observed_at",
            "ci_etag",
        ],
    ),
    ("remote_topic", &["provider", "provider_repo_id", "topic"]),
    (
        "remote_ci_run",
        &[
            "provider",
            "provider_repo_id",
            "run_id",
            "workflow_name",
            "conclusion",
            "branch",
            "run_number",
            "started_at",
        ],
    ),
];

fn primary_key_of(conn: &rusqlite::Connection, table: &str) -> Vec<String> {
    let mut st = conn
        .prepare(&format!("PRAGMA table_info(\"{table}\")"))
        .unwrap();
    let mut members: Vec<(i64, String)> = st
        .query_map([], |r| Ok((r.get::<_, i64>(5)?, r.get::<_, String>(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .filter(|(pk, _)| *pk > 0)
        .collect();
    members.sort_by_key(|(pk, _)| *pk);
    members.into_iter().map(|(_, name)| name).collect()
}

fn table_sql(conn: &rusqlite::Connection, table: &str) -> String {
    conn.query_row(
        "SELECT sql FROM sqlite_schema WHERE type='table' AND name=?1",
        [table],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn the_three_fact_tables_exist_with_their_keys() {
    let (_dir, conn) = migrated_to(9);

    assert_eq!(
        primary_key_of(&conn, "remote_repo"),
        vec!["provider".to_owned(), "provider_repo_id".to_owned()]
    );
    assert_eq!(
        primary_key_of(&conn, "remote_topic"),
        vec![
            "provider".to_owned(),
            "provider_repo_id".to_owned(),
            "topic".to_owned()
        ]
    );
    assert_eq!(
        primary_key_of(&conn, "remote_ci_run"),
        vec![
            "provider".to_owned(),
            "provider_repo_id".to_owned(),
            "run_id".to_owned()
        ]
    );

    assert!(
        table_sql(&conn, "remote_topic").contains("WITHOUT ROWID"),
        "remote_topic is WITHOUT ROWID"
    );
    for (table, _) in FACT_TABLE_COLUMNS {
        assert!(
            table_sql(&conn, table).contains("STRICT"),
            "{table} lost STRICT"
        );
    }

    // The forge owns the conclusion vocabulary, so a closed mirror of it is R26 by construction.
    let ci = table_sql(&conn, "remote_ci_run");
    assert!(
        !ci.contains("CHECK"),
        "remote_ci_run declares a CHECK; conclusion's vocabulary belongs to the forge:\n{ci}"
    );
}

#[test]
fn the_three_fact_tables_carry_their_whole_column_set() {
    let (_dir, conn) = migrated_to(9);
    for (table, expected) in FACT_TABLE_COLUMNS {
        let actual = columns(&conn, table);
        let missing: Vec<&&str> = expected
            .iter()
            .filter(|c| !actual.iter().any(|a| a == **c))
            .collect();
        let extra: Vec<&String> = actual
            .iter()
            .filter(|c| !expected.contains(&c.as_str()))
            .collect();
        assert_eq!(
            actual,
            expected.iter().map(|c| (*c).to_owned()).collect::<Vec<_>>(),
            "{table}: missing {missing:?}, extra {extra:?}"
        );
    }
}

/// §22.11 makes the three fact tables "a fact cache with no identity role" that **references**
/// the binding. A basis column on one of them would put identity on a row §22.9 forbids keying
/// identity on. This is the structural half of the rule Task 8 states for `RemoteBinding::key`,
/// asserted over the migration this plan owns rather than over source it does not.
#[test]
fn no_remote_table_carries_the_link_basis() {
    let (_dir, conn) = migrated_to(9);
    let mut scanned = 0_usize;
    for table in tables(&conn) {
        if !table.starts_with("remote_") {
            continue;
        }
        scanned += 1;
        assert!(
            !columns(&conn, &table)
                .iter()
                .any(|c| c == "remote_link_basis"),
            "{table} carries remote_link_basis; the binding lives on project and nowhere else"
        );
    }
    eprintln!("scanned {scanned} remote_* tables for a link basis");
    assert_eq!(
        scanned, 3,
        "three remote_* tables, or the walk found nothing"
    );
}

/// **AC-P2-25-25's schema-level half.** A rename moves `remote_key`; nothing keys a facts row on
/// it, so the facts and their counts are untouched by one. (§25's behavioural half is p2-25's.)
#[test]
fn a_facts_row_survives_its_project_being_renamed() {
    let (_dir, conn) = migrated_to(9);
    conn.execute(
        "INSERT INTO project (name, seed_basename, remote_key, provider, provider_repo_id,
                              remote_link_basis, created_at, updated_at)
         VALUES ('widget', 'widget', 'forge.example/acme/widget', 'github', '42',
                 'provider_id', 1, 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO remote_repo (provider, provider_repo_id, stars, open_issues, observed_at)
         VALUES ('github', '42', 7, 3, 50)",
        [],
    )
    .unwrap();

    conn.execute(
        "UPDATE project SET remote_key = 'forge.example/acme/gadget' WHERE provider_repo_id='42'",
        [],
    )
    .unwrap();

    let (stars, open_issues, observed_at): (i64, i64, i64) = conn
        .query_row(
            "SELECT stars, open_issues, observed_at FROM remote_repo
              WHERE provider='github' AND provider_repo_id='42'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!((stars, open_issues, observed_at), (7, 3, 50));

    // And the stable id is on `project` alone.
    let elsewhere: Vec<String> = tables(&conn)
        .into_iter()
        .filter(|t| t != "project" && columns(&conn, t).iter().any(|c| c == "provider_repo_id"))
        .collect();
    assert_eq!(
        elsewhere,
        vec![
            "remote_ci_run".to_owned(),
            "remote_repo".to_owned(),
            "remote_topic".to_owned()
        ],
        "only the fact cache references the pair"
    );
}

// ---------------------------------------------------------------------------------------------
// Task 4 — the wire, and the CHECK ↔ enum mirror.
// ---------------------------------------------------------------------------------------------

const MIGRATION_0009: &str = include_str!("../migrations/0009_remote_identity_and_facts.sql");
const SCHEMA_JSON: &str = include_str!("../../protocol/schema/protocol.json");

/// The variants the schema declares. **The schema is the owner**: `core/src/protocol.rs` and
/// `app/src/generated/protocol.ts` are generated from this file and `npm run gen:check` fails if
/// either drifts, so reading it here compares the DDL against the one declaration rather than
/// against a third hand-written copy (R31).
fn schema_variants(name: &str) -> Vec<String> {
    let doc: serde_json::Value = serde_json::from_str(SCHEMA_JSON).unwrap();
    let variants = doc["types"][name]["variants"]
        .as_array()
        .unwrap_or_else(|| panic!("{name} declares no variants in protocol.json"));
    let mut out: Vec<String> = variants
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    assert!(!out.is_empty(), "{name} has no variants to compare");
    out.sort();
    out
}

/// The quoted literals inside `0009`'s `CHECK (<column> IS NULL OR <column> IN (…))`.
fn check_literals(column: &str) -> Vec<String> {
    let needle = format!("CHECK ({column} IS NULL OR {column} IN");
    let start = MIGRATION_0009
        .find(&needle)
        .unwrap_or_else(|| panic!("0009 declares no CHECK for {column}"));
    let rest = &MIGRATION_0009[start + needle.len()..];
    let end = rest
        .find(')')
        .unwrap_or_else(|| panic!("{column}'s CHECK has no closing parenthesis"));
    let mut out: Vec<String> = rest[..end]
        .split('\'')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect();
    assert!(!out.is_empty(), "{column}'s CHECK lists no literals");
    out.sort();
    out
}

/// A column, its CHECK, and the generated enum are one vocabulary. Asserted **in both
/// directions**: a literal the schema does not declare fails, and a variant the column rejects
/// fails. R26 is a DDL CHECK that refuses the values its own core emits.
fn assert_column_mirrors_enum(conn: &rusqlite::Connection, column: &str, enum_name: &str) {
    let declared = schema_variants(enum_name);
    assert_eq!(
        check_literals(column),
        declared,
        "{column}'s CHECK and {enum_name}'s variants are one vocabulary"
    );
    for variant in &declared {
        conn.execute(
            &format!("UPDATE project SET {column} = ?1 WHERE id = 1"),
            [variant],
        )
        .unwrap_or_else(|e| {
            panic!("{column} refused {variant:?}, which {enum_name} declares: {e}")
        });
    }
    eprintln!(
        "{column} accepted all {} {enum_name} variants",
        declared.len()
    );
}

/// **AC-P2-25-10, the DDL-and-enum half.** The chain that *writes* `remote` — manifest, remote,
/// README, note, detected — is p2-25's and is not asserted here.
#[test]
fn every_description_source_variant_is_accepted_by_the_column() {
    let (_dir, conn) = migrated_to(9);
    conn.execute(
        "INSERT INTO project (id, name, seed_basename, created_at, updated_at)
         VALUES (1, 'widget', 'widget', 1, 1)",
        [],
    )
    .unwrap();

    assert_column_mirrors_enum(&conn, "description_source", "DescriptionSource");

    // And the generated Rust takes the same strings off the wire.
    for variant in schema_variants("DescriptionSource") {
        let decoded: codotheca_core::protocol::DescriptionSource =
            serde_json::from_str(&format!("\"{variant}\"")).unwrap();
        assert_eq!(decoded.slug(), variant);
    }
    assert!(
        check_literals("description_source").contains(&"remote".to_owned()),
        "0009 is the migration that widens the CHECK by 'remote'"
    );
}

#[test]
fn every_remote_link_basis_variant_is_accepted_by_the_column() {
    let (_dir, conn) = migrated_to(9);
    conn.execute(
        "INSERT INTO project (id, name, seed_basename, created_at, updated_at)
         VALUES (1, 'widget', 'widget', 1, 1)",
        [],
    )
    .unwrap();

    assert_column_mirrors_enum(&conn, "remote_link_basis", "RemoteLinkBasis");

    // The variant strings ARE the stored strings: codegen emits `#[serde(rename = "<literal>")]`
    // per variant, so `providerId` — the generated Rust identifier — is never a stored value.
    for variant in schema_variants("RemoteLinkBasis") {
        let decoded: codotheca_core::protocol::RemoteLinkBasis =
            serde_json::from_str(&format!("\"{variant}\"")).unwrap();
        assert_eq!(
            serde_json::to_string(&decoded).unwrap(),
            format!("\"{variant}\"")
        );
    }

    // And a value neither side declares is refused rather than stored.
    assert!(conn
        .execute(
            "UPDATE project SET remote_link_basis = 'providerId' WHERE id = 1",
            [],
        )
        .is_err());
}

/// The chain ends where the constant says it does, and the constant is the count the guard
/// checked. Three statements of one value, so a migration registered without its bump is red.
#[test]
fn the_supported_version_is_eleven_and_the_chain_reaches_it() {
    assert_eq!(SUPPORTED_SCHEMA_VERSION, 11);
    assert_eq!(guard_contiguous(MIGRATIONS).unwrap(), 11);
    let (_dir, conn) = migrated_to(MIGRATIONS.len());
    assert_eq!(schema_version(&conn).unwrap(), 11);
}

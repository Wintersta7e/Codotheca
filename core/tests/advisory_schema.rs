//! `0014_advisories.sql` — every CHECK proven by **insertion against a real migrated database**,
//! and a rebuild that keeps its indexes and its live rows.
//!
//! A test that reads the DDL text restates it, and a restatement cannot disagree with the thing it
//! restates. Every list below is derived from the generated enum and never written out as a
//! literal, so a slug that drifts between the schema and the column fails here rather than at a
//! user's first write.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::index::migrate::{
    apply_all, guard_contiguous, schema_version, MIGRATIONS, SUPPORTED_SCHEMA_VERSION,
};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::protocol::{DependencyReadState, Ecosystem, SyncTaskKind};
use codotheca_core::sync::task::kind_slug;

/// The committed contract, compiled in rather than re-found at runtime.
const SCHEMA: &str = include_str!("../../protocol/schema/protocol.json");

/// The generated enum's own spelling, read back through serde rather than restated (R24).
fn slug<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value).expect("a generated enum serialises") {
        serde_json::Value::String(raw) => raw,
        other => panic!("a generated enum did not serialise as a string: {other:?}"),
    }
}

/// Every variant the schema declares for `name`, in declaration order. Panics for an absent type:
/// a missing enum must fail the test, not quietly reduce it to a loop over nothing.
fn schema_variants(name: &str) -> Vec<String> {
    let doc: serde_json::Value = serde_json::from_str(SCHEMA).expect("protocol.json parses");
    let decl = doc["types"]
        .get(name)
        .unwrap_or_else(|| panic!("{name} is not declared in protocol.json"));
    let variants: Vec<String> = decl["variants"]
        .as_array()
        .unwrap_or_else(|| panic!("{name} declares no variants"))
        .iter()
        .map(|v| v.as_str().expect("a variant is a string").to_owned())
        .collect();
    assert!(!variants.is_empty(), "{name} declares no variants");
    variants
}

/// A database at exactly `version` files applied, through the shipped set.
fn migrated_to(version: usize) -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, &MIGRATIONS[..version]).unwrap();
    (dir, conn)
}

fn fresh() -> (tempfile::TempDir, rusqlite::Connection) {
    migrated_to(MIGRATIONS.len())
}

/// One `sync_task_state` row, read column by column. Named rather than a ten-tuple so a mismatch
/// names the column that moved instead of a position.
#[derive(Debug, PartialEq, Eq)]
struct TaskRow {
    id: i64,
    task: String,
    key: Option<i64>,
    state: String,
    cursor: Option<String>,
    fail_count: i64,
    throttle_count: i64,
    reason: Option<String>,
    at: i64,
    not_before: i64,
}

fn read_task_row(conn: &rusqlite::Connection, id: i64) -> rusqlite::Result<TaskRow> {
    conn.query_row(
        "SELECT id, task, key, state, cursor, fail_count, throttle_count, reason, at, not_before
           FROM sync_task_state WHERE id = ?1",
        [id],
        |r| {
            Ok(TaskRow {
                id: r.get(0)?,
                task: r.get(1)?,
                key: r.get(2)?,
                state: r.get(3)?,
                cursor: r.get(4)?,
                fail_count: r.get(5)?,
                throttle_count: r.get(6)?,
                reason: r.get(7)?,
                at: r.get(8)?,
                not_before: r.get(9)?,
            })
        },
    )
}

fn insert_project(conn: &rusqlite::Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES (?1, ?1, 1, 1)",
        [name],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// **AC-P3-32-6.** Every slug the generated enums declare is accepted by the column written from
/// them, proven by inserting each one — and **the number inserted is printed and asserted**, so a
/// run that walked an empty list cannot read as a pass.
///
/// The two lists are the Rust `ALL` consts, and they are checked against the schema's own variant
/// arrays in the same test: a const that fell behind the schema would otherwise make this loop
/// smaller and still green.
#[test]
fn ac_p3_32_6_every_slug_is_accepted_by_its_column() {
    let (_dir, conn) = fresh();
    let project = insert_project(&conn, "p");

    let ecosystems: Vec<String> = Ecosystem::ALL.iter().map(slug).collect();
    let read_states: Vec<String> = DependencyReadState::ALL.iter().map(slug).collect();
    assert_eq!(ecosystems, schema_variants("Ecosystem"));
    assert_eq!(read_states, schema_variants("DependencyReadState"));

    // **The store's own slug functions are the ones the columns are written from**, so they are
    // read back against serde here rather than trusted: a total `match` that drifted from the
    // generated spelling would write a value its own CHECK refuses, on a user's machine.
    for eco in Ecosystem::ALL {
        assert_eq!(codotheca_core::advisories::eco_slug(eco), slug(&eco));
    }
    for state in DependencyReadState::ALL {
        assert_eq!(
            codotheca_core::advisories::read_state_slug(state),
            slug(&state)
        );
    }

    let mut inserted = 0usize;
    for (i, eco) in ecosystems.iter().enumerate() {
        conn.execute(
            "INSERT INTO project_lockfile
               (project_id, source_path, ecosystem, read_state, size_bytes, observed_at)
             VALUES (?1, ?2, ?3, 'parsed', 1, 1)",
            rusqlite::params![project, format!("eco/{i}/lock"), eco],
        )
        .unwrap_or_else(|e| panic!("the CHECK rejected ecosystem={eco}: {e}"));
        conn.execute(
            "INSERT INTO project_dependency
               (project_id, ecosystem, package_name, version, source_path, observed_at)
             VALUES (?1, ?2, 'pkg', '1.0.0', 'lock', 1)",
            rusqlite::params![project, eco],
        )
        .unwrap_or_else(|e| panic!("project_dependency rejected ecosystem={eco}: {e}"));
        inserted += 2;
    }
    for (i, state) in read_states.iter().enumerate() {
        conn.execute(
            "INSERT INTO project_lockfile
               (project_id, source_path, ecosystem, read_state, size_bytes, observed_at)
             VALUES (?1, ?2, 'npm', ?3, NULL, 1)",
            rusqlite::params![project, format!("state/{i}/lock"), state],
        )
        .unwrap_or_else(|e| panic!("the CHECK rejected read_state={state}: {e}"));
        inserted += 1;
    }

    eprintln!(
        "advisory_schema: {} ecosystem slugs, {} read-state slugs, {inserted} rows inserted",
        ecosystems.len(),
        read_states.len()
    );
    assert_eq!(inserted, ecosystems.len() * 2 + read_states.len());
    assert!(inserted > 0, "a run that inserted nothing proved nothing");

    // The sweep this producer writes carries `worktree`, which is the third of AC-P3-32-6 that is
    // discharged against §28's own column rather than against a column this migration adds.
    let basis_ok: i64 = conn
        .query_row(
            "SELECT count(*) FROM pragma_table_info('debt_sweep') WHERE name = 'basis'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        basis_ok, 1,
        "the basis this read states lives on debt_sweep"
    );
}

/// A slug no enum declares is **refused**, by each of the three columns that carry one. Without
/// this, the CHECK could be `IN (<anything>)` and the test above would still pass.
#[test]
fn a_slug_no_enum_declares_is_refused_by_every_column() {
    let (_dir, conn) = fresh();
    let project = insert_project(&conn, "p");

    let lockfile_eco = conn.execute(
        "INSERT INTO project_lockfile
           (project_id, source_path, ecosystem, read_state, size_bytes, observed_at)
         VALUES (?1, 'a/lock', 'npmjs', 'parsed', 1, 1)",
        rusqlite::params![project],
    );
    assert!(lockfile_eco.is_err(), "'npmjs' is not an ecosystem");

    let lockfile_state = conn.execute(
        "INSERT INTO project_lockfile
           (project_id, source_path, ecosystem, read_state, size_bytes, observed_at)
         VALUES (?1, 'b/lock', 'npm', 'skipped', 1, 1)",
        rusqlite::params![project],
    );
    assert!(lockfile_state.is_err(), "'skipped' is not a read state");

    let dependency_eco = conn.execute(
        "INSERT INTO project_dependency
           (project_id, ecosystem, package_name, version, source_path, observed_at)
         VALUES (?1, 'npmjs', 'p', '1', 'lock', 1)",
        rusqlite::params![project],
    );
    assert!(dependency_eco.is_err(), "'npmjs' is not an ecosystem");
}

/// **AC-P3-32-23.** The rebuild widens the task vocabulary, **keeps both partial indexes**, and
/// **carries the live rows across**.
///
/// The parked row is written *before* the migration and read back column by column *after* it:
/// the failure this guards is a user's in-flight listing cursor discarded on upgrade, which no
/// error reports and no later run can reconstruct.
#[test]
fn ac_p3_32_23_the_rebuild_keeps_its_indexes_and_its_rows() {
    let before = MIGRATIONS.len() - 1;
    let dir = tempfile::tempdir().unwrap();
    let path = Index::db_path(dir.path());
    let mut conn = open_connection(&path).unwrap();
    apply_all(&mut conn, &MIGRATIONS[..before]).unwrap();

    conn.execute(
        "INSERT INTO sync_task_state
           (id, task, key, state, cursor, fail_count, throttle_count, reason, at, not_before)
         VALUES (7, 'account_repos', 41, 'parked', 'cursor-page-3', 2, 3, 'reserve', 900, 1800)",
        [],
    )
    .unwrap();

    apply_all(&mut conn, MIGRATIONS).unwrap();
    assert_eq!(schema_version(&conn).unwrap(), SUPPORTED_SCHEMA_VERSION);

    let carried = read_task_row(&conn, 7).expect("the parked row survived the rebuild");
    assert_eq!(
        carried,
        TaskRow {
            id: 7,
            task: "account_repos".to_owned(),
            key: Some(41),
            state: "parked".to_owned(),
            cursor: Some("cursor-page-3".to_owned()),
            fail_count: 2,
            throttle_count: 3,
            reason: Some("reserve".to_owned()),
            at: 900,
            not_before: 1800,
        }
    );

    // The widened CHECK admits every kind the generated enum declares, including the fourth.
    let mut accepted = 0usize;
    for kind in SyncTaskKind::ALL {
        let key = if kind == SyncTaskKind::Advisories {
            None
        } else {
            Some(99)
        };
        conn.execute(
            "INSERT INTO sync_task_state (task, key, state, at) VALUES (?1, ?2, 'queued', 1)",
            rusqlite::params![kind_slug(kind), key],
        )
        .unwrap_or_else(|e| panic!("the widened CHECK rejected {}: {e}", kind_slug(kind)));
        accepted += 1;
    }
    eprintln!("advisory_schema: {accepted} task slugs accepted after the rebuild");
    assert_eq!(accepted, SyncTaskKind::ALL.len());

    // `sync_task_global` is the slot the advisory task occupies. Two NULL-key rows for one task
    // must not coexist: `put`'s `ON CONFLICT(task) WHERE key IS NULL` has no index to name if the
    // rebuild dropped it, and the runner would see two rows for one task.
    let second_global = conn.execute(
        "INSERT INTO sync_task_state (task, key, state, at) VALUES ('advisories', NULL, 'queued', 2)",
        [],
    );
    assert!(
        second_global.is_err(),
        "a second key-less advisories row was admitted: sync_task_global is missing"
    );

    // `sync_task_keyed` is the other half and is recreated by the same file.
    let second_keyed = conn.execute(
        "INSERT INTO sync_task_state (task, key, state, at) VALUES ('account_repos', 99, 'queued', 2)",
        [],
    );
    assert!(
        second_keyed.is_err(),
        "a duplicate (task, key) row was admitted: sync_task_keyed is missing"
    );
}

/// R68's half for this file: the chain reaches 14 with no hole, and the constant a start-up guard
/// compares against agrees with it.
#[test]
fn the_migration_chain_reaches_fourteen_with_no_hole() {
    assert_eq!(guard_contiguous(MIGRATIONS).unwrap(), 14);
    assert_eq!(SUPPORTED_SCHEMA_VERSION, 14);
    assert_eq!(
        MIGRATIONS.last().map(|m| m.version),
        Some(SUPPORTED_SCHEMA_VERSION)
    );
    let (_dir, conn) = fresh();
    assert_eq!(schema_version(&conn).unwrap(), 14);
}

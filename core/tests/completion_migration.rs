//! `0015_completion.sql` — every CHECK proven by **insertion against a real migrated database**,
//! and the one column `location` gained proven by a diff over its whole column list.
//!
//! A test that reads the DDL text restates it, and a restatement cannot disagree with the thing
//! it restates. Every list below is enumerated from the generated enum and never written out as a
//! literal, so a slug that drifts between the schema and the column fails here rather than at a
//! user's first write.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;

use codotheca_core::index::migrate::{
    apply_all, schema_version, MIGRATIONS, SUPPORTED_SCHEMA_VERSION,
};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::protocol::{CheckState, CompletionCheck, UnknownReason};

/// The generated enum's own spelling, read back through serde rather than restated (R24).
fn slug<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value).expect("a generated enum serialises") {
        serde_json::Value::String(raw) => raw,
        other => panic!("a generated enum did not serialise as a string: {other:?}"),
    }
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

/// One project to hang rows off. `project_check.project_id` is a real foreign key, so a test
/// that invented an id would prove the CHECKs against a row the schema would refuse anyway.
fn project(conn: &rusqlite::Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES (?1, ?1, 1, 1)",
        [name],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn insert_check(
    conn: &rusqlite::Connection,
    id: i64,
    key: &str,
    state: &str,
    reason: Option<&str>,
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO project_check (project_id, check_key, state, user_na, unknown_reason,
                                    observed_at)
         VALUES (?1, ?2, ?3, NULL, ?4, 100)",
        rusqlite::params![id, key, state, reason],
    )
}

/// The whole column list of one table, in declaration order.
fn columns(conn: &rusqlite::Connection, table: &str) -> Vec<String> {
    let mut st = conn
        .prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
        .unwrap();
    let rows = st
        .query_map([table], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    rows
}

/// `0015` stamps fifteen. [p3] `0016` moved the chain's tip, so this file's own version is read
/// off the chain at fifteen rather than off the tip, and the tip is compared to the constant.
#[test]
fn the_completion_migration_stamps_fifteen() {
    let (_dir, conn) = migrated_to(15);
    assert_eq!(schema_version(&conn).unwrap(), 15);
    assert_eq!(MIGRATIONS[14].name, "completion");
    // The literal and the table move together, which is the only thing that keeps a build from
    // refusing a database it wrote itself.
    assert_eq!(
        MIGRATIONS[MIGRATIONS.len() - 1].version,
        SUPPORTED_SCHEMA_VERSION
    );
}

/// Every `CheckState` variant is insertable, and a word that is not one is refused.
///
/// The set is `CheckState::ALL`, never a literal four: a fifth state added to the schema without
/// the CHECK fails here, and a CHECK listing a state the enum does not have fails the converse
/// assertion below.
#[test]
fn every_check_state_and_every_check_key_inserts() {
    let (_dir, conn) = fresh();
    let id = project(&conn, "states");

    let mut inserted = 0_u32;
    for (i, state) in CheckState::ALL.iter().enumerate() {
        let text = slug(state);
        // The paired CHECK forces a reason on `unknown` and forbids one everywhere else, so the
        // fixture is built from the state rather than around it.
        let reason = (text == "unknown").then(|| slug(&UnknownReason::NotRead));
        let key = slug(&CompletionCheck::ALL[i]);
        insert_check(&conn, id, &key, &text, reason.as_deref())
            .unwrap_or_else(|e| panic!("state {text:?} is not insertable: {e}"));
        inserted += 1;
    }
    eprintln!("check states inserted: {inserted}");
    assert_eq!(
        inserted, 4,
        "a run that inserts zero states is a failing run"
    );
    assert!(inserted > 0);

    // And every one of the ten keys, which is the other half of the same CHECK pair.
    let keyed = project(&conn, "keys");
    let mut keys = 0_u32;
    for key in CompletionCheck::ALL {
        insert_check(&conn, keyed, &slug(&key), "pass", None)
            .unwrap_or_else(|e| panic!("key {:?} is not insertable: {e}", slug(&key)));
        keys += 1;
    }
    eprintln!("check keys inserted: {keys}");
    assert_eq!(keys, 10);

    // A word neither list holds.
    assert!(insert_check(&conn, id, "readme", "maybe", None).is_err());
    assert!(insert_check(&conn, keyed, "coverage", "pass", None).is_err());
}

/// Every `UnknownReason` variant is insertable, enumerated from the generated enum.
///
/// R129/F2: §30's prose claims §31's mirror *"stands at four"*. It stands at six, and this test
/// is what makes a four-variant CHECK fail on the day it is written rather than after a second
/// rebuild of a WITHOUT ROWID table.
#[test]
fn every_unknown_reason_inserts_and_a_stranger_does_not() {
    let (_dir, conn) = fresh();
    let id = project(&conn, "reasons");

    let mut inserted = 0_u32;
    for (i, reason) in UnknownReason::ALL.iter().enumerate() {
        let text = slug(reason);
        insert_check(
            &conn,
            id,
            &slug(&CompletionCheck::ALL[i]),
            "unknown",
            Some(&text),
        )
        .unwrap_or_else(|e| panic!("unknown_reason {text:?} is not insertable: {e}"));
        inserted += 1;
    }
    eprintln!("unknown reasons inserted: {inserted}");
    assert!(
        inserted > 0,
        "a run that inserts zero reasons is a failing run"
    );
    assert_eq!(usize::try_from(inserted).unwrap(), UnknownReason::ALL.len());

    let stored: BTreeSet<String> = {
        let mut st = conn
            .prepare("SELECT unknown_reason FROM project_check WHERE project_id = ?1")
            .unwrap();
        st.query_map([id], |r| r.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let declared: BTreeSet<String> = UnknownReason::ALL.iter().map(slug).collect();
    assert_eq!(stored, declared, "the CHECK accepted a different set");

    assert!(insert_check(&conn, id, "deps", "unknown", Some("needsGithub")).is_err());
}

/// §31.1: an `unknown` check is counted AND named. Both halves of the pairing are refused.
#[test]
fn the_paired_check_refuses_both_halves_of_its_violation() {
    let (_dir, conn) = fresh();
    let id = project(&conn, "paired");

    // `unknown` with no reason.
    assert!(insert_check(&conn, id, "readme", "unknown", None).is_err());
    // A reason on a state that was evaluated. A `fail` that names a reason it could not be read
    // is the invariant failing quietly, which is exactly what this half exists to catch.
    assert!(insert_check(&conn, id, "readme", "fail", Some("notRead")).is_err());
    assert!(insert_check(&conn, id, "readme", "pass", Some("notRead")).is_err());
    assert!(insert_check(&conn, id, "readme", "na", Some("notRead")).is_err());

    // And the two legal shapes, so the CHECK is proven to admit as well as to refuse.
    insert_check(&conn, id, "readme", "unknown", Some("notRead")).unwrap();
    insert_check(&conn, id, "license", "fail", None).unwrap();
}

/// `user_na` is three-valued and the CHECK says so: NULL, 0 and 1, and nothing else.
#[test]
fn user_na_is_three_valued() {
    let (_dir, conn) = fresh();
    let id = project(&conn, "user_na");
    for (key, value) in [("readme", None), ("license", Some(0)), ("tests", Some(1))] {
        conn.execute(
            "INSERT INTO project_check (project_id, check_key, state, user_na, unknown_reason,
                                        observed_at)
             VALUES (?1, ?2, 'pass', ?3, NULL, 100)",
            rusqlite::params![id, key, value],
        )
        .unwrap_or_else(|e| panic!("user_na {value:?} is not insertable: {e}"));
    }
    assert!(conn
        .execute(
            "INSERT INTO project_check (project_id, check_key, state, user_na, unknown_reason,
                                        observed_at)
             VALUES (?1, 'ci', 'pass', 2, NULL, 100)",
            [id],
        )
        .is_err());
}

/// §31.5: the primary key serves the only read, so a second row for one `(project, key)` pair is
/// refused rather than silently doubling a count.
#[test]
fn one_row_per_project_and_key() {
    let (_dir, conn) = fresh();
    let id = project(&conn, "pk");
    insert_check(&conn, id, "deps", "pass", None).unwrap();
    assert!(insert_check(&conn, id, "deps", "fail", None).is_err());
}

/// §31.2's column, asserted as a **diff over the whole column list** rather than by a
/// `has_column` probe.
///
/// A probe answers *is `tag_count` there*, which is true of a migration that also dropped three
/// other columns. The diff answers *what changed*, which is the claim.
#[test]
fn location_gained_exactly_one_column_and_it_is_tag_count() {
    // [p3] Pinned to `0015`'s own step: the chain's tip moved past it, and a tip-relative pair
    // would diff whichever migration happens to be last.
    let (_before_dir, before) = migrated_to(14);
    let (_after_dir, after) = migrated_to(15);

    let was = columns(&before, "location");
    let now = columns(&after, "location");
    eprintln!(
        "location columns: {} before 0015, {} after",
        was.len(),
        now.len()
    );
    assert!(
        !was.is_empty(),
        "a run that read zero columns is a failing run"
    );

    let added: Vec<&String> = now.iter().filter(|c| !was.contains(c)).collect();
    let removed: Vec<&String> = was.iter().filter(|c| !now.contains(c)).collect();
    assert_eq!(added, [&"tag_count".to_owned()], "added: {added:?}");
    assert!(removed.is_empty(), "0015 removed a column: {removed:?}");
    // The order of what was already there is unchanged, which is what distinguishes an ALTER
    // from a create-copy-drop-rename that happened to end with the same set.
    assert_eq!(now[..was.len()], was[..]);

    // Nullable with no default: `0` would claim currency the app does not have.
    let (notnull, dflt): (i64, Option<String>) = after
        .query_row(
            "SELECT \"notnull\", dflt_value FROM pragma_table_info('location')
              WHERE name = 'tag_count'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(notnull, 0);
    assert_eq!(dflt, None);
}

/// R59, asserted over the file the build compiles in rather than over a copy of it.
#[test]
fn the_migration_names_no_pragma() {
    let sql = include_str!("../migrations/0015_completion.sql");
    let lines = sql.lines().count();
    let hits: Vec<(usize, &str)> = sql
        .lines()
        .enumerate()
        // The prose above the DDL explains WHY there is no PRAGMA, so the scan is over statements
        // and not over the whole file — a comment naming the thing it forbids is the reason the
        // rule survives, and banning the word from the comments would delete it.
        .filter(|(_, line)| !line.trim_start().starts_with("--"))
        .filter(|(_, line)| line.to_ascii_uppercase().contains("PRAGMA"))
        .map(|(i, line)| (i + 1, line))
        .collect();
    eprintln!("0015_completion.sql: {lines} lines scanned for PRAGMA");
    assert!(lines > 0, "a run that scanned zero lines is a failing run");
    assert!(hits.is_empty(), "R59: {hits:?}");

    // And the file is the one the migration table points at, not a stray copy.
    let entry = MIGRATIONS
        .iter()
        .find(|m| m.version == 15)
        .expect("0015 is registered");
    assert_eq!(entry.name, "completion");
    assert_eq!(entry.sql, sql);
    assert!(
        !entry.rebuilds_a_table,
        "0015 copies no table, so the runner must not toggle foreign keys for it"
    );
}

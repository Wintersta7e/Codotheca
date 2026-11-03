//! `0021_badges.sql` — §40.7's `badge_award`, latched by the schema itself: one row per kind, ever;
//! `project_id` is the only column that moves and a landing is written once. Plus the section
//! adapters the sidecar registry calls.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::badges::store::{export_section, restore_section};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::sidecar::{RestoreCtx, RestoreOutcome};
use codotheca_core::index::subject::ProjectSubject;
use codotheca_core::index::{open_connection, Index};
use codotheca_core::protocol::BadgeKind;
use rusqlite::types::Value;
use rusqlite::Connection;

const NOW: i64 = 1_760_000_000;

/// A database with every shipped migration applied, and the directory that holds it.
fn fresh() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

/// A live project holding `lineage`, with no remote.
fn project(conn: &Connection, name: &str, lineage: &str) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
         VALUES (?1, ?1, ?2, 1, 1)",
        [name, lineage],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn lineage_key(lineage: &str) -> String {
    ProjectSubject::Lineage {
        lineage_key: lineage.to_owned(),
        remote_key: None,
    }
    .to_key()
}

/// An unlanded award of `kind`, naming `project` and `subject` when given.
fn award(conn: &Connection, kind: &str, project: Option<i64>, subject: Option<&str>) {
    conn.execute(
        "INSERT INTO badge_award (kind, earned_at, tz_offset_min, earned_on, project_id,
                                  subject_key, evidence, awarded_at, awarded_on)
         VALUES (?1, 86400, 0, '1970-01-02', ?2, ?3, '{}', 90000, '1970-01-02')",
        rusqlite::params![kind, project, subject],
    )
    .unwrap();
}

/// Every column of the award of `kind`, or `None` when the kind is not held.
fn row_of(conn: &Connection, kind: &str) -> Option<Vec<Value>> {
    let mut st = conn
        .prepare("SELECT * FROM badge_award WHERE kind = ?1")
        .unwrap();
    let width = st.column_count();
    let mut rows = st
        .query_map([kind], |r| {
            (0..width).map(|i| r.get::<_, Value>(i)).collect()
        })
        .unwrap();
    rows.next().map(Result::unwrap)
}

fn project_id_of(conn: &Connection, kind: &str) -> Option<i64> {
    conn.query_row(
        "SELECT project_id FROM badge_award WHERE kind = ?1",
        [kind],
        |r| r.get(0),
    )
    .unwrap()
}

fn set_recursive(conn: &Connection, on: bool) {
    conn.pragma_update(None, "recursive_triggers", on).unwrap();
}

#[test]
fn ac_p4_40_2_a_delete_aborts() {
    let (_dir, conn) = fresh();
    award(&conn, "necromancer", None, None);
    let error = conn.execute("DELETE FROM badge_award", []).unwrap_err();
    eprintln!("delete: {error}");
    assert!(
        error.to_string().contains("badge_award is latched"),
        "{error}"
    );
    assert!(
        row_of(&conn, "necromancer").is_some(),
        "the row is still there"
    );
}

#[test]
fn ac_p4_40_2_replace_and_upsert_leave_a_held_row_byte_identical() {
    let (_dir, conn) = fresh();
    award(&conn, "polyglot", None, None);
    let held = row_of(&conn, "polyglot").unwrap();
    let mut tried = 0;
    for recursive in [false, true] {
        set_recursive(&conn, recursive);
        conn.execute(
            "INSERT OR REPLACE INTO badge_award (kind, earned_at, tz_offset_min, earned_on,
                                                evidence, awarded_at, awarded_on)
             VALUES ('polyglot', 1, 60, '2001-01-01', '{\"year\":2001}', 2, '2001-01-01')",
            [],
        )
        .unwrap();
        assert_eq!(
            row_of(&conn, "polyglot").unwrap(),
            held,
            "INSERT OR REPLACE, recursive_triggers {recursive}"
        );
        conn.execute(
            "INSERT INTO badge_award (kind, earned_at, tz_offset_min, earned_on, evidence,
                                      awarded_at, awarded_on)
             VALUES ('polyglot', 1, 60, '2001-01-01', '{\"year\":2001}', 2, '2001-01-01')
             ON CONFLICT(kind) DO UPDATE SET evidence = excluded.evidence,
                                             earned_on = excluded.earned_on",
            [],
        )
        .unwrap();
        assert_eq!(
            row_of(&conn, "polyglot").unwrap(),
            held,
            "ON CONFLICT DO UPDATE, recursive_triggers {recursive}"
        );
        tried += 2;
    }
    eprintln!("{tried} rewrites tried, the row byte-identical after each");
}

#[test]
fn ac_p4_40_2_only_the_project_moves_and_a_landing_is_written_once() {
    let (_dir, conn) = fresh();
    let first = project(&conn, "first", "aa");
    let second = project(&conn, "second", "bb");
    award(
        &conn,
        "marathon",
        Some(first),
        Some(lineage_key("aa").as_str()),
    );
    let columns: Vec<(String, String)> = conn
        .prepare("SELECT name, type FROM pragma_table_info('badge_award')")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    eprintln!("{} columns: {columns:?}", columns.len());
    assert_ne!(columns.len(), 0, "enumerated no column");

    let mut refused = 0;
    for (column, ty) in &columns {
        if matches!(column.as_str(), "project_id" | "landed_at" | "landed_on") {
            continue;
        }
        // A different value of the column's own type; `kind` moves to another real kind.
        let other = match (column.as_str(), ty.as_str()) {
            ("kind", _) => "'comeback'",
            (_, "INTEGER") => "12345",
            _ => "'other'",
        };
        let error = conn
            .execute(&format!("UPDATE badge_award SET {column} = {other}"), [])
            .unwrap_err();
        assert!(
            error.to_string().contains("badge_award is latched"),
            "{column}: {error}"
        );
        refused += 1;
    }
    eprintln!("{refused} column rewrites refused");
    assert_eq!(refused, columns.len() - 3);

    conn.execute("UPDATE badge_award SET project_id = ?1", [second])
        .unwrap();
    assert_eq!(project_id_of(&conn, "marathon"), Some(second));
    conn.execute("UPDATE badge_award SET project_id = NULL", [])
        .unwrap();
    assert_eq!(project_id_of(&conn, "marathon"), None);

    // One landed column without the other fails the CHECK.
    for half in ["landed_at = 100", "landed_on = '1970-01-02'"] {
        let error = conn
            .execute(&format!("UPDATE badge_award SET {half}"), [])
            .unwrap_err();
        assert!(error.to_string().contains("CHECK"), "{half}: {error}");
    }
    conn.execute(
        "UPDATE badge_award SET landed_at = 100, landed_on = '1970-01-02'",
        [],
    )
    .unwrap();
    for again in [
        "landed_at = 200, landed_on = '1970-01-03'",
        "landed_at = 200",
        "landed_on = '1970-01-03'",
        "landed_at = NULL, landed_on = NULL",
    ] {
        let error = conn
            .execute(&format!("UPDATE badge_award SET {again}"), [])
            .unwrap_err();
        assert!(
            error.to_string().contains("badge_award is latched"),
            "{again}: {error}"
        );
    }
    // The project still moves after a landing.
    conn.execute("UPDATE badge_award SET project_id = ?1", [first])
        .unwrap();
    assert_eq!(project_id_of(&conn, "marathon"), Some(first));
}

#[test]
fn ac_p4_40_2_deleting_a_referenced_project_aborts_and_deletes_nothing() {
    let (_dir, conn) = fresh();
    let named = project(&conn, "named", "cc");
    award(&conn, "necromancer", Some(named), None);
    let held = row_of(&conn, "necromancer").unwrap();
    for recursive in [false, true] {
        set_recursive(&conn, recursive);
        let error = conn
            .execute("DELETE FROM project WHERE id = ?1", [named])
            .unwrap_err();
        eprintln!("recursive_triggers {recursive}: {error}");
        assert!(
            error.to_string().contains("badge_award is latched"),
            "{error}"
        );
        let projects: i64 = conn
            .query_row("SELECT COUNT(*) FROM project WHERE id = ?1", [named], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(projects, 1, "the project row remains");
        assert_eq!(
            row_of(&conn, "necromancer").unwrap(),
            held,
            "the award remains"
        );
    }
}

#[test]
fn ac_p4_40_18_the_kind_check_is_the_generated_vocabulary() {
    let (_dir, conn) = fresh();
    let create: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'badge_award'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let (_, after) = create.split_once("kind IN (").expect("kind's CHECK");
    let (list, _) = after.split_once(')').unwrap();
    let checked: Vec<String> = list
        .split(',')
        .map(|v| v.trim().trim_matches('\'').to_owned())
        .collect();
    let generated: Vec<String> = BadgeKind::ALL
        .iter()
        .map(|k| {
            serde_json::to_value(k)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    eprintln!("CHECK {checked:?}; generated {generated:?}");
    assert_eq!(checked, generated, "the CHECK and BadgeKind differ");

    let mut inserted = 0;
    for kind in &generated {
        award(&conn, kind, None, None);
        inserted += 1;
    }
    eprintln!("{inserted} inserted");
    assert_ne!(inserted, 0, "inserted no kind");
    let error = conn
        .execute(
            "INSERT INTO badge_award (kind, earned_at, tz_offset_min, earned_on, evidence,
                                      awarded_at, awarded_on)
             VALUES ('demolitionist', 1, 0, '1970-01-01', '{}', 1, '1970-01-01')",
            [],
        )
        .unwrap_err();
    assert!(error.to_string().contains("CHECK"), "{error}");
}

/// The registry's adapters: a `Global` section answers `Applied` and never `Pending`, and maps a
/// subject to a project only when exactly one live project holds it — never the lowest id.
#[test]
fn the_section_adapters_answer_applied_and_never_pending() {
    let (_src_dir, src) = fresh();
    let held = project(&src, "held", "dd");
    award(&src, "necromancer", Some(held), Some(&lineage_key("dd")));
    award(&src, "comeback", None, Some(&lineage_key("nobody")));
    award(&src, "curator", None, Some(&lineage_key("twice")));
    let rows = export_section(&src).unwrap();
    eprintln!("{} section rows exported", rows.len());
    assert_eq!(rows.len(), 3, "exported every award");
    for row in &rows {
        assert_eq!(row.subject, None, "a Global row carries no subject");
        assert!(row.location_keys.is_empty(), "the table names no location");
    }

    let (_dir, mut conn) = fresh();
    let unique = project(&conn, "unique", "dd");
    let lowest = project(&conn, "one", "twice");
    project(&conn, "two", "twice");
    let ctx = RestoreCtx {
        now: NOW,
        project: None,
        locations: &[],
        source_generation: 1,
    };
    let restore = |db: &mut Connection, row| {
        let tx = db.transaction().unwrap();
        let outcome = restore_section(&tx, row, &ctx).unwrap();
        tx.commit().unwrap();
        outcome
    };
    for row in &rows {
        assert_eq!(restore(&mut conn, row), RestoreOutcome::Applied(1));
        let kind = row.data["kind"].as_str().unwrap();
        let written = row_of(&conn, kind).unwrap();
        assert_eq!(restore(&mut conn, row), RestoreOutcome::Applied(0));
        assert_eq!(row_of(&conn, kind).unwrap(), written, "{kind} rewritten");
    }
    assert_eq!(project_id_of(&conn, "necromancer"), Some(unique));
    assert_eq!(project_id_of(&conn, "comeback"), None, "no holder");
    assert_eq!(
        project_id_of(&conn, "curator"),
        None,
        "two holders, and the lowest is {lowest}"
    );
}

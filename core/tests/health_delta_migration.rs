//! `0016_health_delta.sql` — the one `health_delta` rebuild: the `(project_id, ts)` index, the
//! `layer` CHECK mirroring `DecayLayer`, and the cascade the table never had (§34.3).
//!
//! **The CHECK is a cross-language mirror, so it is tested by reading the other side** (R24,
//! R26): the variants come out of `protocol/schema/protocol.json`, never out of a literal here.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;

use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use rusqlite::{params, Connection};

/// The committed contract, compiled in rather than re-found at runtime.
const SCHEMA: &str = include_str!("../../protocol/schema/protocol.json");

/// The migration's own text, compiled in: what it states is what the chain ran.
const MIGRATION: &str = include_str!("../migrations/0016_health_delta.sql");

/// A database at exactly `version` files applied, through the shipped set.
fn migrated_to(version: usize) -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, &MIGRATIONS[..version]).unwrap();
    (dir, conn)
}

fn fresh() -> (tempfile::TempDir, Connection) {
    migrated_to(MIGRATIONS.len())
}

/// Every variant the schema declares for `name`, in declaration order.
///
/// Panics rather than returning an empty vector for an absent type: a missing enum must fail the
/// test, not silently reduce it to a loop over nothing.
fn schema_variants(name: &str) -> Vec<String> {
    let schema: serde_json::Value = serde_json::from_str(SCHEMA).unwrap();
    let declaration = schema["types"]
        .get(name)
        .unwrap_or_else(|| panic!("{name} is not declared in protocol.json"));
    assert_eq!(declaration["kind"], "enum", "{name} is not an enum");
    let variants: Vec<String> = declaration["variants"]
        .as_array()
        .unwrap_or_else(|| panic!("{name} declares no variants"))
        .iter()
        .map(|variant| variant.as_str().unwrap().to_owned())
        .collect();
    assert!(!variants.is_empty(), "{name} declares no variants");
    variants
}

fn insert_project(conn: &Connection) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES ('sample', 'sample', 1, 1)",
        [],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn insert_delta(conn: &Connection, project: i64, layer: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO health_delta (project_id, ts, layer, from_value, to_value, detected_in)
         VALUES (?1, 42, ?2, 3.0, 2.0, 'foreground')",
        params![project, layer],
    )
}

#[derive(Debug, PartialEq, Eq)]
struct Column {
    name: String,
    declared_type: String,
    not_null: bool,
    primary_key: i64,
}

fn columns(conn: &Connection) -> Vec<Column> {
    conn.prepare("PRAGMA table_info(health_delta)")
        .unwrap()
        .query_map([], |row| {
            Ok(Column {
                name: row.get(1)?,
                declared_type: row.get(2)?,
                not_null: row.get(3)?,
                primary_key: row.get(5)?,
            })
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The three changes are present, and **no column moved**: the list is diffed against the one
/// the chain produced at fifteen, never restated — so `from_value` and `to_value` staying `REAL`
/// (§34.9's *not superseded* row) is checked by the comparison rather than by a literal.
#[test]
fn ac_p3_34_11_rebuild_preserves_columns_and_adds_index_and_cascade() {
    let (_before_dir, before) = migrated_to(15);
    let (_after_dir, after) = fresh();
    let version: u32 = after
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 16);
    let old_columns = columns(&before);
    let new_columns = columns(&after);
    eprintln!("health_delta columns compared: {}", old_columns.len());
    assert!(!old_columns.is_empty());
    assert_eq!(old_columns, new_columns);

    let indexes: Vec<String> = after
        .prepare("PRAGMA index_list(health_delta)")
        .unwrap()
        .query_map([], |r| r.get(1))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    eprintln!("health_delta indexes scanned: {}", indexes.len());
    assert!(indexes
        .iter()
        .any(|name| name == "idx_health_delta_project_ts"));
    let indexed_columns: Vec<String> = after
        .prepare("PRAGMA index_info(idx_health_delta_project_ts)")
        .unwrap()
        .query_map([], |r| r.get(2))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(indexed_columns, ["project_id", "ts"]);

    let keys: Vec<(String, String, String, String)> = after
        .prepare("PRAGMA foreign_key_list(health_delta)")
        .unwrap()
        .query_map([], |r| Ok((r.get(2)?, r.get(3)?, r.get(4)?, r.get(6)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        keys,
        [(
            "project".to_owned(),
            "project_id".to_owned(),
            "id".to_owned(),
            "CASCADE".to_owned()
        )]
    );
}

/// The mirror, read from the schema: one row per `DecayLayer` variant is accepted, and the
/// prototype's internal keys — §27.7's drift against exactly this column — are refused.
#[test]
fn ac_p3_34_11_layer_check_mirrors_schema_and_rejects_prototype_keys() {
    let (_dir, conn) = fresh();
    let project = insert_project(&conn);
    let variants = schema_variants("DecayLayer");
    let mut inserted = 0;
    for variant in &variants {
        inserted += insert_delta(&conn, project, variant)
            .unwrap_or_else(|error| panic!("layer {variant} was refused: {error}"));
    }
    eprintln!("DecayLayer variants inserted: {inserted}");
    assert!(
        inserted > 0,
        "a mirror that inserted nothing proved nothing"
    );
    assert_eq!(inserted, variants.len());
    for invalid in ["web", "growth"] {
        assert!(
            insert_delta(&conn, project, invalid).is_err(),
            "the CHECK accepted {invalid}"
        );
    }
}

/// **The fixture is bare, and the name says so because the guarantee is narrow.** The project
/// holds nothing but `health_delta` rows. Other `project` children — `location`, `session`,
/// `xp_events` and more — still carry no cascade, so a real project with a copy on disk is still
/// refused by `DELETE FROM project`: this proves `health_delta` no longer blocks the delete, and
/// nothing about deleting a project.
#[test]
fn ac_p3_34_11_bare_fixture_cascades_without_a_project_deletion_guarantee() {
    let (_dir, conn) = fresh();
    let enabled: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        enabled, 1,
        "a cascade test with foreign keys off tests nothing"
    );
    let project = insert_project(&conn);
    insert_delta(&conn, project, "rust").unwrap();
    insert_delta(&conn, project, "dust").unwrap();
    conn.execute("DELETE FROM project WHERE id = ?1", [project])
        .unwrap();
    let remaining: i64 = conn
        .query_row("SELECT count(*) FROM health_delta", [], |r| r.get(0))
        .unwrap();
    eprintln!("bare fixture: 2 deltas inserted, {remaining} remain");
    assert_eq!(remaining, 0);
}

/// `id` is `AUTOINCREMENT`: a table whose rows were all deleted before the rebuild must still
/// never hand an old id out again.
#[test]
fn ac_p3_34_11_deleted_rows_keep_the_sequence_high_water_mark() {
    let (_dir, mut conn) = migrated_to(15);
    let project = insert_project(&conn);
    for _ in 0..3 {
        insert_delta(&conn, project, "rust").unwrap();
    }
    let highest = conn.last_insert_rowid();
    conn.execute("DELETE FROM health_delta", []).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 16);
    let sequence: i64 = conn
        .query_row(
            "SELECT seq FROM sqlite_sequence WHERE name = 'health_delta'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(sequence, highest);
    insert_delta(&conn, project, "dust").unwrap();
    assert!(conn.last_insert_rowid() > highest);
}

/// Rows written before the rebuild come across unchanged, ids included.
#[test]
fn ac_p3_34_11_existing_rows_survive_the_copy() {
    let (_dir, mut conn) = migrated_to(15);
    let project = insert_project(&conn);
    insert_delta(&conn, project, "rust").unwrap();
    let id = conn.last_insert_rowid();
    let snapshot = |db: &Connection| -> Vec<serde_json::Value> {
        db.prepare(
            "SELECT id, project_id, ts, layer, from_value, to_value, detected_in
               FROM health_delta ORDER BY id",
        )
        .unwrap()
        .query_map([], |r| {
            Ok(serde_json::json!([
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<f64>>(4)?,
                r.get::<_, Option<f64>>(5)?,
                r.get::<_, String>(6)?
            ]))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
    };
    let before = snapshot(&conn);
    apply_all(&mut conn, MIGRATIONS).unwrap();
    let after = snapshot(&conn);
    eprintln!("health_delta rows compared: {}", before.len());
    assert!(!before.is_empty());
    assert_eq!(before, after);
    insert_delta(&conn, project, "dust").unwrap();
    assert!(conn.last_insert_rowid() > id);
}

fn without_comments(sql: &str) -> String {
    let mut chars = sql.chars().peekable();
    let mut stripped = String::new();
    while let Some(ch) = chars.next() {
        if ch == '-' && chars.peek() == Some(&'-') {
            for comment in chars.by_ref() {
                if comment == '\n' {
                    break;
                }
            }
            stripped.push(' ');
        } else if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(comment) = chars.next() {
                if comment == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
            stripped.push(' ');
        } else {
            stripped.push(ch);
        }
    }
    stripped
}

/// **R59, read off the file.** `apply_all` wraps each migration in a transaction where
/// `PRAGMA foreign_keys` is a documented no-op; a statement written here would do nothing and read
/// as if it did. Comments are stripped first, because the header explains the rule in words.
#[test]
fn ac_p3_34_11_migration_leaves_pragma_to_the_runner() {
    let statements = without_comments(MIGRATION);
    eprintln!(
        "migration bytes scanned: {}, without comments: {}",
        MIGRATION.len(),
        statements.len()
    );
    assert!(!statements.trim().is_empty());
    assert!(!statements
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|token| token.eq_ignore_ascii_case("pragma")));
}

/// §28.8's enumeration is stated once, in this migration, **and this is what keeps it true**: the
/// names the comment lists are compared against the migrated schema's own cascading children. A
/// later migration that adds one without updating the list fails here rather than leaving a
/// comment nobody reads.
#[test]
fn ac_p3_34_11_cascade_comment_matches_the_live_set() {
    let documented: BTreeSet<String> = MIGRATION
        .lines()
        .filter_map(|line| line.strip_prefix("-- cascade-child: ").map(str::to_owned))
        .collect();
    let (_dir, conn) = fresh();
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    eprintln!(
        "tables scanned for cascading project references: {}",
        tables.len()
    );
    assert!(!tables.is_empty());
    let mut live = BTreeSet::new();
    for table in tables {
        let mut statement = conn
            .prepare("SELECT \"table\", on_delete FROM pragma_foreign_key_list(?1)")
            .unwrap();
        let keys = statement
            .query_map([&table], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .unwrap();
        for key in keys {
            let (parent, on_delete) = key.unwrap();
            if parent == "project" && on_delete == "CASCADE" {
                live.insert(table.clone());
            }
        }
    }
    eprintln!("documented cascades: {documented:?}\nlive cascades: {live:?}");
    assert!(!documented.is_empty());
    assert!(!live.is_empty());
    assert_eq!(documented, live);
}

//! `0018_recovering_half.sql` — `project.removed_at` and the four removal tables (§46.15), read off
//! a real migrated database. No writer lands with them; these tests hold the shape later writers
//! rely on.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::index::migrate::{apply_all, MIGRATIONS, SUPPORTED_SCHEMA_VERSION};
use codotheca_core::index::{open_connection, Index};
use rusqlite::types::Value;
use rusqlite::{params_from_iter, Connection};

/// The migration's own text, compiled in: what it states is what the chain ran.
const MIGRATION: &str = include_str!("../migrations/0018_recovering_half.sql");

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

/// A column as the DDL states it: name, declared type, NOT NULL, primary-key position.
type ColumnSpec = (&'static str, &'static str, bool, i64);

/// A foreign key as the DDL states it: the column, the table it references, its `ON DELETE`.
type KeySpec = (&'static str, &'static str, &'static str);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Column {
    name: String,
    declared_type: String,
    not_null: bool,
    default: Option<String>,
    primary_key: i64,
}

fn columns(conn: &Connection, table: &str) -> Vec<Column> {
    conn.prepare("SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info(?1)")
        .unwrap()
        .query_map([table], |row| {
            Ok(Column {
                name: row.get(0)?,
                declared_type: row.get(1)?,
                not_null: row.get(2)?,
                default: row.get(3)?,
                primary_key: row.get(4)?,
            })
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

/// Inserts one row built from `defaults` with each `overrides` entry replacing its column (or
/// added when the defaults do not name it), and returns the new rowid.
fn insert(
    conn: &Connection,
    table: &str,
    defaults: &[(&str, Value)],
    overrides: &[(&str, Value)],
) -> rusqlite::Result<i64> {
    let mut row: Vec<(&str, Value)> = defaults.to_vec();
    for (name, value) in overrides {
        match row.iter_mut().find(|(column, _)| column == name) {
            Some(slot) => slot.1 = value.clone(),
            None => row.push((name, value.clone())),
        }
    }
    let names: Vec<&str> = row.iter().map(|(name, _)| *name).collect();
    let marks: Vec<String> = (1..=row.len()).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "INSERT INTO {table} ({}) VALUES ({})",
        names.join(", "),
        marks.join(", ")
    );
    conn.execute(&sql, params_from_iter(row.iter().map(|(_, value)| value)))?;
    Ok(conn.last_insert_rowid())
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

fn insert_location(conn: &Connection, project: i64, path: &str) -> i64 {
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind)
         VALUES (?1, 'linux', ?2, ?2, ?3, 'store-a', 'present', 'worktree')",
        rusqlite::params![project, path.as_bytes(), path],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// A `preserving` parcel with every NOT NULL column filled and no seal.
fn parcel_defaults(project: i64, location: i64) -> Vec<(&'static str, Value)> {
    vec![
        ("project_id", Value::Integer(project)),
        ("location_id", Value::Integer(location)),
        ("state", text("preserving")),
        ("folder_bytes", Value::Blob(b"/preserved".to_vec())),
        (
            "staging_bytes",
            Value::Blob(b"/preserved/.staging".to_vec()),
        ),
        ("lineage_key", text("lineage:sample")),
        ("session_nonce", Value::Blob(vec![7; 16])),
        ("created_at", Value::Integer(1)),
    ]
}

/// The four columns the seal CHECK requires of a `sealed` or `purged` parcel.
fn seal() -> Vec<(&'static str, Value)> {
    vec![
        ("state_digest", text("digest")),
        ("manifest_sha256", text("sha256")),
        ("dir_name", text("parcel-1")),
        ("sealed_at", Value::Integer(2)),
    ]
}

/// A `done` record recovered from a remote, with every NOT NULL column filled.
fn record_defaults(project: i64, location: i64) -> Vec<(&'static str, Value)> {
    vec![
        ("project_id", Value::Integer(project)),
        ("location_id", Value::Integer(location)),
        ("kind", text("uninstall")),
        ("state", text("done")),
        ("path_bytes", Value::Blob(b"/copies/sample".to_vec())),
        ("planned", text("trash")),
        ("lineage_key", text("lineage:sample")),
        ("state_digest", text("digest")),
        ("recovery", text("remote")),
        ("session_nonce", Value::Blob(vec![9; 16])),
        ("started_at", Value::Integer(1)),
    ]
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[test]
fn the_chain_reaches_eighteen_and_the_file_is_not_a_rebuild() {
    let migration = &MIGRATIONS[17];
    eprintln!(
        "migration 18: {} (rebuild: {})",
        migration.name, migration.rebuilds_a_table
    );
    assert_eq!(migration.version, 18);
    assert_eq!(migration.name, "recovering_half");
    assert!(
        !migration.rebuilds_a_table,
        "0018 copies, drops and renames nothing, so the runner must not toggle foreign keys"
    );

    let (_dir, conn) = fresh();
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, SUPPORTED_SCHEMA_VERSION);
    assert_eq!(SUPPORTED_SCHEMA_VERSION, MIGRATIONS.last().unwrap().version);
}

/// As one set difference over the whole list, never per-column equalities: a per-column test
/// passes while another column silently vanishes.
#[test]
fn project_gains_exactly_removed_at() {
    let (_before_dir, before) = migrated_to(17);
    let (_after_dir, after) = migrated_to(18);
    let old = columns(&before, "project");
    let new = columns(&after, "project");
    eprintln!("project columns: {} at 17, {} at 18", old.len(), new.len());
    assert_ne!(old, Vec::new(), "no project column read at 17");

    let added: Vec<&Column> = new.iter().filter(|c| !old.contains(c)).collect();
    let removed: Vec<&Column> = old.iter().filter(|c| !new.contains(c)).collect();
    assert_eq!(
        removed,
        Vec::<&Column>::new(),
        "0018 moved a project column"
    );
    assert_eq!(
        added,
        [&Column {
            name: "removed_at".to_owned(),
            declared_type: "INTEGER".to_owned(),
            not_null: false,
            default: None,
            primary_key: 0,
        }]
    );

    let old_location = columns(&before, "location");
    eprintln!("location columns compared: {}", old_location.len());
    assert_ne!(old_location, Vec::new(), "no location column read at 17");
    assert_eq!(old_location, columns(&after, "location"));
}

#[test]
fn the_four_tables_have_the_section_columns() {
    let (_dir, conn) = fresh();
    let expected: [(&str, &[ColumnSpec]); 4] = [
        (
            "parcel",
            &[
                ("id", "INTEGER", false, 1),
                ("project_id", "INTEGER", true, 0),
                ("location_id", "INTEGER", true, 0),
                ("state", "TEXT", true, 0),
                ("folder_bytes", "BLOB", true, 0),
                ("dir_name", "TEXT", false, 0),
                ("staging_bytes", "BLOB", true, 0),
                ("volume_key", "TEXT", false, 0),
                ("store_class", "TEXT", false, 0),
                ("lineage_key", "TEXT", true, 0),
                ("state_digest", "TEXT", false, 0),
                ("manifest_sha256", "TEXT", false, 0),
                ("total_bytes", "INTEGER", false, 0),
                ("git_version", "TEXT", false, 0),
                ("tar_version", "TEXT", false, 0),
                ("session_nonce", "BLOB", true, 0),
                ("created_at", "INTEGER", true, 0),
                ("sealed_at", "INTEGER", false, 0),
                ("checked_at", "INTEGER", false, 0),
                ("full_checked_at", "INTEGER", false, 0),
                ("full_checked_git", "TEXT", false, 0),
                ("check_result", "TEXT", true, 0),
                ("purged_at", "INTEGER", false, 0),
            ],
        ),
        (
            "parcel_ref",
            &[
                ("parcel_id", "INTEGER", true, 1),
                ("repo_path", "BLOB", true, 2),
                ("ref_name", "TEXT", true, 3),
                ("oid", "TEXT", true, 0),
            ],
        ),
        (
            "removal_record",
            &[
                ("id", "INTEGER", false, 1),
                ("project_id", "INTEGER", true, 0),
                ("location_id", "INTEGER", true, 0),
                ("kind", "TEXT", true, 0),
                ("state", "TEXT", true, 0),
                ("path_bytes", "BLOB", true, 0),
                ("planned", "TEXT", true, 0),
                ("holding_bytes", "BLOB", false, 0),
                ("lineage_key", "TEXT", true, 0),
                ("state_digest", "TEXT", true, 0),
                ("recovery", "TEXT", true, 0),
                ("remotes_json", "TEXT", false, 0),
                ("remote_verified_at", "INTEGER", false, 0),
                ("parcel_id", "INTEGER", false, 0),
                ("disposal", "TEXT", false, 0),
                ("readme_name", "TEXT", false, 0),
                ("readme_text", "TEXT", false, 0),
                ("readme_truncated", "INTEGER", false, 0),
                ("session_nonce", "BLOB", true, 0),
                ("started_at", "INTEGER", true, 0),
                ("ended_at", "INTEGER", false, 0),
            ],
        ),
        (
            "removal_log",
            &[
                ("removal_id", "INTEGER", false, 1),
                ("format", "INTEGER", true, 0),
                ("log", "TEXT", true, 0),
            ],
        ),
    ];
    let mut compared = 0_usize;
    for (table, want) in expected {
        let got: Vec<(String, String, bool, i64)> = columns(&conn, table)
            .into_iter()
            .map(|c| (c.name, c.declared_type, c.not_null, c.primary_key))
            .collect();
        let want: Vec<(String, String, bool, i64)> = want
            .iter()
            .map(|(n, t, nn, pk)| ((*n).to_owned(), (*t).to_owned(), *nn, *pk))
            .collect();
        assert_eq!(got, want, "{table}'s columns");
        let sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |r| r.get(0),
            )
            .unwrap();
        assert!(sql.contains("STRICT"), "{table} is not STRICT: {sql}");
        compared += got.len();
    }
    eprintln!("columns compared across the four tables: {compared}");
    assert_eq!(compared, 51);
}

#[test]
fn foreign_keys_cascade_where_the_section_says() {
    let (_dir, conn) = fresh();
    let expected: [(&str, &[KeySpec]); 4] = [
        (
            "parcel",
            &[
                ("location_id", "location", "NO ACTION"),
                ("project_id", "project", "CASCADE"),
            ],
        ),
        ("parcel_ref", &[("parcel_id", "parcel", "CASCADE")]),
        (
            "removal_record",
            &[
                ("location_id", "location", "NO ACTION"),
                ("parcel_id", "parcel", "NO ACTION"),
                ("project_id", "project", "CASCADE"),
            ],
        ),
        (
            "removal_log",
            &[("removal_id", "removal_record", "CASCADE")],
        ),
    ];
    let mut compared = 0_usize;
    for (table, want) in expected {
        let mut got: Vec<(String, String, String)> = conn
            .prepare("SELECT \"from\", \"table\", on_delete FROM pragma_foreign_key_list(?1)")
            .unwrap()
            .query_map([table], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        got.sort();
        let want: Vec<(String, String, String)> = want
            .iter()
            .map(|(f, t, d)| ((*f).to_owned(), (*t).to_owned(), (*d).to_owned()))
            .collect();
        assert_eq!(got, want, "{table}'s foreign keys");
        compared += got.len();
    }
    eprintln!("foreign keys compared: {compared}");
    assert_eq!(compared, 7);
}

/// An unchecked parcel never reads as checked, and never as nothing.
#[test]
fn check_result_defaults_to_unchecked_and_is_never_null() {
    let (_dir, conn) = fresh();
    let project = insert_project(&conn);
    let location = insert_location(&conn, project, "/copies/sample");
    let parcel = insert(&conn, "parcel", &parcel_defaults(project, location), &[]).unwrap();
    let stored: String = conn
        .query_row(
            "SELECT check_result FROM parcel WHERE id = ?1",
            [parcel],
            |r| r.get(0),
        )
        .unwrap();
    eprintln!("check_result of a parcel written without one: {stored}");
    assert_eq!(stored, "unchecked");
    assert!(
        insert(
            &conn,
            "parcel",
            &parcel_defaults(project, location),
            &[("check_result", Value::Null)],
        )
        .is_err(),
        "the column accepted NULL"
    );
}

#[test]
fn a_sealed_or_purged_parcel_carries_its_seal() {
    let (_dir, conn) = fresh();
    let project = insert_project(&conn);
    let location = insert_location(&conn, project, "/copies/sample");
    let defaults = parcel_defaults(project, location);
    let mut refused = 0_u32;
    let mut accepted = 0_u32;
    for state in ["sealed", "purged"] {
        for missing in seal().iter().map(|(name, _)| *name) {
            let mut row: Vec<(&str, Value)> =
                seal().into_iter().filter(|(n, _)| *n != missing).collect();
            row.push(("state", text(state)));
            if insert(&conn, "parcel", &defaults, &row).is_err() {
                refused += 1;
            } else {
                eprintln!("a {state} parcel was accepted without {missing}");
            }
        }
        let mut row = seal();
        row.push(("state", text(state)));
        insert(&conn, "parcel", &defaults, &row)
            .unwrap_or_else(|e| panic!("a {state} parcel with its seal was refused: {e}"));
        accepted += 1;
    }
    for state in ["preserving", "abandoned"] {
        insert(&conn, "parcel", &defaults, &[("state", text(state))])
            .unwrap_or_else(|e| panic!("a {state} parcel without a seal was refused: {e}"));
        accepted += 1;
    }
    eprintln!("seal CHECK: {refused} refused, {accepted} accepted");
    assert_eq!(refused, 8);
    assert_eq!(accepted, 4);
}

#[test]
fn recovery_names_a_parcel_exactly_when_it_is_parcel() {
    let (_dir, conn) = fresh();
    let project = insert_project(&conn);
    let first = insert_location(&conn, project, "/copies/first");
    let second = insert_location(&conn, project, "/copies/second");
    let parcel = insert(&conn, "parcel", &parcel_defaults(project, first), &[]).unwrap();

    let parcel_without_id = insert(
        &conn,
        "removal_record",
        &record_defaults(project, first),
        &[("recovery", text("parcel"))],
    );
    let remote_with_id = insert(
        &conn,
        "removal_record",
        &record_defaults(project, first),
        &[("parcel_id", Value::Integer(parcel))],
    );
    eprintln!(
        "parcel without an id refused: {}, remote with an id refused: {}",
        parcel_without_id.is_err(),
        remote_with_id.is_err()
    );
    assert!(
        parcel_without_id.is_err(),
        "recovery 'parcel' named no parcel"
    );
    assert!(remote_with_id.is_err(), "recovery 'remote' named a parcel");

    insert(
        &conn,
        "removal_record",
        &record_defaults(project, first),
        &[
            ("recovery", text("parcel")),
            ("parcel_id", Value::Integer(parcel)),
        ],
    )
    .unwrap();
    insert(
        &conn,
        "removal_record",
        &record_defaults(project, second),
        &[],
    )
    .unwrap();
    assert_eq!(count(&conn, "removal_record"), 2);
}

#[test]
fn one_open_record_per_location() {
    let (_dir, conn) = fresh();
    let project = insert_project(&conn);
    let record = |location: i64, state: &str| {
        insert(
            &conn,
            "removal_record",
            &record_defaults(project, location),
            &[("state", text(state))],
        )
    };
    let mut refused = 0_u32;

    let two_journaled = insert_location(&conn, project, "/copies/a");
    record(two_journaled, "journaled").unwrap();
    if record(two_journaled, "journaled").is_err() {
        refused += 1;
    }

    let journaled_then_interrupted = insert_location(&conn, project, "/copies/b");
    record(journaled_then_interrupted, "journaled").unwrap();
    if record(journaled_then_interrupted, "interrupted").is_err() {
        refused += 1;
    }

    let done_then_journaled = insert_location(&conn, project, "/copies/c");
    record(done_then_journaled, "done").unwrap();
    record(done_then_journaled, "journaled")
        .unwrap_or_else(|e| panic!("a closed record blocked a new open one: {e}"));

    eprintln!("second open records refused: {refused} of 2");
    assert_eq!(refused, 2);
}

#[test]
fn lineage_is_required_on_both_tables() {
    let (_dir, conn) = fresh();
    let project = insert_project(&conn);
    let location = insert_location(&conn, project, "/copies/sample");
    let null_lineage = [("lineage_key", Value::Null)];
    let parcel = insert(
        &conn,
        "parcel",
        &parcel_defaults(project, location),
        &null_lineage,
    );
    let record = insert(
        &conn,
        "removal_record",
        &record_defaults(project, location),
        &null_lineage,
    );
    eprintln!(
        "NULL lineage refused: parcel {}, removal_record {}",
        parcel.is_err(),
        record.is_err()
    );
    assert!(parcel.is_err(), "a parcel was written with no lineage");
    assert!(
        record.is_err(),
        "a removal record was written with no lineage"
    );
    // The same rows with a lineage are accepted, so the refusals above are the column's and not
    // a table that does not exist.
    insert(&conn, "parcel", &parcel_defaults(project, location), &[]).unwrap();
    insert(
        &conn,
        "removal_record",
        &record_defaults(project, location),
        &[],
    )
    .unwrap();
}

/// The `project` cascade is asserted by `foreign_key_list` only. A `location` row always stands
/// between a project and its parcels and records, and `location` does not cascade, so deleting
/// the project is refused — which this test also shows, so nobody reads it as a deletion path.
#[test]
fn deleting_a_parcel_takes_its_refs_and_deleting_a_record_takes_its_log() {
    let (_dir, conn) = fresh();
    let enabled: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        enabled, 1,
        "a cascade test with foreign keys off tests nothing"
    );
    let project = insert_project(&conn);
    let location = insert_location(&conn, project, "/copies/sample");

    let parcel = insert(&conn, "parcel", &parcel_defaults(project, location), &[]).unwrap();
    for (repo, name) in [(&b""[..], "HEAD"), (&b"vendor/lib"[..], "refs/heads/main")] {
        conn.execute(
            "INSERT INTO parcel_ref (parcel_id, repo_path, ref_name, oid) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                parcel,
                repo,
                name,
                "0123456789abcdef0123456789abcdef01234567"
            ],
        )
        .unwrap();
    }
    let record = insert(
        &conn,
        "removal_record",
        &record_defaults(project, location),
        &[],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO removal_log (removal_id, format, log) VALUES (?1, 1, 'header')",
        [record],
    )
    .unwrap();
    let refs_before = count(&conn, "parcel_ref");
    let logs_before = count(&conn, "removal_log");

    assert!(
        conn.execute("DELETE FROM project WHERE id = ?1", [project])
            .is_err(),
        "the project was deleted although a location still names it"
    );

    conn.execute("DELETE FROM parcel WHERE id = ?1", [parcel])
        .unwrap();
    conn.execute("DELETE FROM removal_record WHERE id = ?1", [record])
        .unwrap();
    let refs_after = count(&conn, "parcel_ref");
    let logs_after = count(&conn, "removal_log");
    eprintln!(
        "parcel_ref {refs_before} -> {refs_after}, removal_log {logs_before} -> {logs_after}"
    );
    assert_eq!((refs_before, logs_before), (2, 1));
    assert_eq!((refs_after, logs_after), (0, 0));
}

/// Rows written at seventeen read identically at eighteen over the seventeen columns, and none of
/// them is removed: there is no backfill.
#[test]
fn existing_project_rows_survive_with_removed_at_null() {
    let (_dir, mut conn) = migrated_to(17);
    insert_project(&conn);
    insert_project(&conn);
    let names: Vec<String> = columns(&conn, "project")
        .into_iter()
        .map(|c| c.name)
        .collect();
    let select = format!("SELECT {} FROM project ORDER BY id", names.join(", "));
    let snapshot = |db: &Connection| -> Vec<Vec<Value>> {
        db.prepare(&select)
            .unwrap()
            .query_map([], |row| {
                (0..names.len())
                    .map(|i| row.get::<_, Value>(i))
                    .collect::<rusqlite::Result<Vec<Value>>>()
            })
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let before = snapshot(&conn);
    apply_all(&mut conn, MIGRATIONS).unwrap();
    let after = snapshot(&conn);
    eprintln!(
        "project rows compared: {} over {} columns",
        before.len(),
        names.len()
    );
    assert_eq!(before.len(), 2);
    assert_eq!(before, after);
    let removed: i64 = conn
        .query_row(
            "SELECT count(*) FROM project WHERE removed_at IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(removed, 0);
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

/// `apply_all` wraps each migration in a transaction where `PRAGMA foreign_keys` is a documented
/// no-op; a statement written here would do nothing and read as if it did. Comments are stripped
/// first, because the header explains the rule in words.
#[test]
fn the_migration_leaves_pragma_to_the_runner() {
    let statements = without_comments(MIGRATION);
    let tokens = statements
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|token| !token.is_empty())
        .count();
    eprintln!(
        "migration bytes scanned: {}, tokens without comments: {tokens}",
        MIGRATION.len()
    );
    assert!(tokens > 0, "the migration states nothing");
    assert!(!statements
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|token| token.eq_ignore_ascii_case("pragma")));
}

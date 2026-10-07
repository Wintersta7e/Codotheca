//! The format-2 export (§48.8.1): its schema version, every location's key, a removed location's
//! whole row, each session's copy, the pending rows verbatim, the section registry, and the
//! registration gate that holds every table to a classification (§48.8.5).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;
use std::path::Path;

use codotheca_core::index::migrate::{apply_all, Migration, MIGRATIONS, SUPPORTED_SCHEMA_VERSION};
use codotheca_core::index::sidecar::{
    counts, export, inspect, unclassified, unclassified_in, write_atomically, Classification,
    Entry, SidecarLocationKey, SidecarState, SidecarValue, CLASSIFIED, SECTIONS, SIDECAR_FORMAT,
};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::testing::sidecar::SECTION_FIXTURES;
use rusqlite::Connection;

fn migrated(dir: &Path) -> Connection {
    let mut conn = open_connection(&Index::db_path(dir)).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    conn
}

/// One lineage project with two copies: `/r/a` present, `/r/b` removed by this app.
fn two_copies(conn: &Connection) {
    conn.execute_batch(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
         VALUES ('thing', 'thing', 'l1', 1, 1);
         INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind)
         VALUES (1, 'linux', x'2f722f61', x'2f722f61', '/r/a', 's', 'present', 'worktree');
         INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind, removed_at, branch)
         VALUES (1, 'linux', x'2f722f62', x'2f722f62', '/r/b', 's', 'missing', 'worktree',
                 500, 'main');",
    )
    .unwrap();
}

fn key(path_key: &str) -> SidecarLocationKey {
    SidecarLocationKey {
        kind: "linux".to_owned(),
        distro: String::new(),
        path_key: path_key.to_owned(),
    }
}

#[test]
fn the_export_writes_format_2_with_its_schema_version() {
    let dir = tempfile::tempdir().unwrap();
    let conn = migrated(dir.path());
    let doc = export(&conn, 1, 1_000).unwrap();
    assert_eq!(doc.format, SIDECAR_FORMAT);
    assert_eq!(doc.format, 2);
    assert_eq!(doc.schema_version, Some(SUPPORTED_SCHEMA_VERSION));

    let path = Index::sidecar_path(dir.path());
    write_atomically(&doc, &path).unwrap();
    match inspect(&path, SUPPORTED_SCHEMA_VERSION) {
        SidecarState::Present(back) => assert_eq!(*back, doc),
        other => panic!("this build's own document read as {other:?}"),
    }
}

/// A removed copy is the one no scan can re-derive, so it travels whole —
/// every column, `removed_at` and the observed facts included — while every copy travels its key.
#[test]
fn every_location_travels_and_a_removed_one_travels_whole() {
    let dir = tempfile::tempdir().unwrap();
    let conn = migrated(dir.path());
    two_copies(&conn);
    let doc = export(&conn, 1, 1_000).unwrap();
    let project = &doc.payload.projects[0];

    assert_eq!(
        project.location_keys,
        vec![key("2f722f61"), key("2f722f62")]
    );
    assert_eq!(project.removed_locations.len(), 1);
    let row = &project.removed_locations[0];
    let columns: i64 = conn
        .query_row(
            "SELECT count(*) FROM pragma_table_info('location')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    eprintln!(
        "removed location columns carried: {} of {columns}",
        row.len()
    );
    assert_eq!(i64::try_from(row.len()).unwrap(), columns);
    assert_eq!(row["removed_at"], SidecarValue::Integer(500));
    assert_eq!(row["path_bytes"], SidecarValue::Blob("2f722f62".to_owned()));
    assert_eq!(row["branch"], SidecarValue::Text("main".to_owned()));
}

#[test]
fn a_session_carries_its_location_key() {
    let dir = tempfile::tempdir().unwrap();
    let conn = migrated(dir.path());
    two_copies(&conn);
    conn.execute_batch(
        "INSERT INTO session (project_id, location_id, started_at, ended_at, credited_seconds,
                              close_reason)
         VALUES (1, 2, 100, 200, 90, 'stop');
         INSERT INTO session (project_id, started_at, ended_at, credited_seconds, close_reason)
         VALUES (1, 300, 400, 90, 'stop');",
    )
    .unwrap();
    let doc = export(&conn, 1, 1_000).unwrap();
    let sessions = &doc.payload.projects[0].sessions;
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].location_key, Some(key("2f722f62")));
    assert_eq!(
        sessions[1].location_key, None,
        "a session that recorded no copy carries none"
    );
}

/// §48.8.4: every export carries the unconsumed pending rows, so a second corruption before
/// every subject returns loses none of the first's — in `(source_generation, id)` order.
#[test]
fn the_export_carries_every_pending_row_verbatim() {
    let dir = tempfile::tempdir().unwrap();
    let conn = migrated(dir.path());
    conn.execute_batch(
        "INSERT INTO sidecar_pending (source_generation, subject_key, location_keys, record,
                                      queued_at)
         VALUES (4, 'lineage:b|remote:', '[]', '{\"b\":1}', 20);
         INSERT INTO sidecar_pending (source_generation, subject_key, location_keys, record,
                                      queued_at)
         VALUES (3, 'path:linux::2f61', '[{\"kind\":\"linux\"}]', '{\"a\":1}', 10);
         INSERT INTO sidecar_pending (source_generation, subject_key, location_keys, record,
                                      queued_at)
         VALUES (4, 'lineage:c|remote:', '[]', 'not even json', 30);",
    )
    .unwrap();
    let doc = export(&conn, 9, 1_000).unwrap();
    let carried: Vec<(i64, &str, &str, &str, i64)> = doc
        .payload
        .pending
        .iter()
        .map(|p| {
            (
                p.source_generation,
                p.subject_key.as_str(),
                p.location_keys.as_str(),
                p.record.as_str(),
                p.queued_at,
            )
        })
        .collect();
    eprintln!("pending rows carried: {}", carried.len());
    assert_eq!(
        carried,
        vec![
            (
                3,
                "path:linux::2f61",
                "[{\"kind\":\"linux\"}]",
                "{\"a\":1}",
                10
            ),
            (4, "lineage:b|remote:", "[]", "{\"b\":1}", 20),
            (4, "lineage:c|remote:", "[]", "not even json", 30),
        ]
    );
    assert_eq!(counts(&doc)["pending"], 3);
}

/// Every registered section can be populated by a test. Zero equals zero until the first section
/// registers; the equality is the point here, and the registration gate is what fails at zero.
#[test]
fn every_registered_section_has_a_fixture() {
    let registered: BTreeSet<&str> = SECTIONS.iter().map(|s| s.name).collect();
    let fixtured: BTreeSet<&str> = SECTION_FIXTURES.iter().map(|(name, _)| *name).collect();
    eprintln!(
        "sections registered: {}, fixtured: {}",
        registered.len(),
        fixtured.len()
    );
    assert_eq!(
        registered.len(),
        SECTIONS.len(),
        "a section registered twice"
    );
    assert_eq!(registered, fixtured);
}

/// AC-P4-48-15: every table the schema holds and every `project` and `location` column is
/// classified, and every classification names something the schema holds.
#[test]
fn ac_p4_48_15_every_table_and_column_is_classified() {
    let dir = tempfile::tempdir().unwrap();
    let conn = migrated(dir.path());
    let report = unclassified(&conn).unwrap_or_else(|findings| panic!("{findings:#?}"));
    eprintln!(
        "classified {} tables and {} columns",
        report.tables, report.columns
    );
    assert_ne!(report.tables, 0, "the gate saw no table");
    assert_ne!(report.columns, 0, "the gate saw no column");
}

/// A table that lands without a classification fails the gate, by name.
#[test]
fn an_unclassified_scratch_table_fails_the_gate() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    let mut migrations = MIGRATIONS.to_vec();
    migrations.push(Migration {
        version: SUPPORTED_SCHEMA_VERSION + 1,
        name: "scratch_unclassified",
        sql: "CREATE TABLE scratch_unclassified (x INTEGER) STRICT;",
        rebuilds_a_table: false,
    });
    apply_all(&mut conn, &migrations).unwrap();
    let findings = unclassified(&conn).unwrap_err();
    eprintln!("{findings:#?}");
    assert_eq!(findings, ["scratch_unclassified: classified by no entry"]);
}

/// An entry for a table the schema no longer holds fails the gate: a stale list is a lying one.
#[test]
fn a_stale_entry_fails_the_gate() {
    let dir = tempfile::tempdir().unwrap();
    let conn = migrated(dir.path());
    conn.execute_batch("DROP TABLE health_delta;").unwrap();
    let findings = unclassified(&conn).unwrap_err();
    eprintln!("{findings:#?}");
    assert_eq!(
        findings,
        ["health_delta: classified, but the schema holds no such table"]
    );
}

/// A section the registry does not hold, a registered section no entry names, and one entry
/// classified the same way twice each fail the gate.
#[test]
fn a_misnamed_unnamed_or_repeated_section_fails_the_gate() {
    let dir = tempfile::tempdir().unwrap();
    let conn = migrated(dir.path());
    let consent = Classification::Section("readme_consent");
    let mut list: Vec<(Entry, Classification)> = CLASSIFIED
        .iter()
        .copied()
        .filter(|(_, class)| *class != consent)
        .collect();
    list.push((
        Entry::Column("project", "readme_remote_at"),
        Classification::Section("readme_grant"),
    ));
    list.push((
        Entry::Table("project"),
        Classification::Section("no_scan_projects"),
    ));
    let findings = unclassified_in(&conn, &list).unwrap_err();
    eprintln!("{findings:#?}");
    assert_eq!(
        findings,
        [
            "project.readme_remote_at: classified under section `readme_grant`, which is not \
             registered",
            "project: classified as Section(\"no_scan_projects\") twice",
            "section `readme_consent`: registered, but no entry is classified under it",
        ]
    );
}

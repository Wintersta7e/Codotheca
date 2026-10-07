//! The sidecar reader's four states, and the format-1 document it must still restore (§48.8.2).
//!
//! `fixtures/sidecar-format1.json` was written by the format-1 exporter itself — the record set
//! `v0.9.0` ships, field for field — never by hand. Its seeded shape, all synthetic: two lineage
//! projects (one pinned and archived, with a note, a session of two segments, one session-track
//! XP row and one custom launch target; one with no remote), one path-subject project, a manual
//! collection holding both lineage projects, one root at `/fixture/repos` (hex), an identity with
//! an alias, `app_meta` `level_floor = '3'`, `first_run_completed_at`, `wsl_consented_distros`,
//! `sidecar_written_at` and `restore_pending_generation`, and one `view_state` row.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use codotheca_core::index::migrate::{apply_all, MIGRATIONS, SUPPORTED_SCHEMA_VERSION};
use codotheca_core::index::sidecar::{
    dump_row, export, insert_row, inspect, restore_global, write_atomically, Sidecar, SidecarRow,
    SidecarState, SidecarValue, REBUILD_OWNED_SETTINGS, RETIRED_SETTINGS,
};
use codotheca_core::index::{open_connection, Index};
use rusqlite::Connection;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sidecar-format1.json")
}

fn migrated(dir: &Path) -> Connection {
    let mut conn = open_connection(&Index::db_path(dir)).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    conn
}

/// A real document from this build's exporter, written to the sidecar path.
fn written(dir: &Path) -> (PathBuf, Sidecar) {
    let conn = migrated(dir);
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, notes, created_at, updated_at)
         VALUES ('thing', 'thing', 'l1', 'a note', 1, 1)",
        [],
    )
    .unwrap();
    let doc = export(&conn, 3, 1_000).unwrap();
    let path = Index::sidecar_path(dir);
    write_atomically(&doc, &path).unwrap();
    (path, doc)
}

fn app_meta(conn: &Connection) -> BTreeMap<String, String> {
    conn.prepare("SELECT k, v FROM app_meta")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// `Absent` is the one I/O error that means "no file"; a directory where the file should be is a
/// file the reader could not read, and must never pass for a missing one.
#[test]
fn a_missing_file_is_absent_and_a_directory_in_its_place_is_unreadable() {
    let dir = tempfile::tempdir().unwrap();
    let path = Index::sidecar_path(dir.path());
    assert!(matches!(
        inspect(&path, SUPPORTED_SCHEMA_VERSION),
        SidecarState::Absent
    ));

    std::fs::create_dir(&path).unwrap();
    match inspect(&path, SUPPORTED_SCHEMA_VERSION) {
        SidecarState::Unreadable { reason } => {
            eprintln!("a directory at the path reads as: {reason}");
            assert_ne!(reason, "");
        }
        other => panic!("a directory in the file's place read as {other:?}"),
    }
}

#[test]
fn a_checksum_mismatch_is_unreadable_and_the_file_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = written(dir.path());
    let text = std::fs::read_to_string(&path).unwrap();
    let tampered = text.replace("a note", "a NOTE");
    assert_ne!(
        tampered, text,
        "the fixture must actually change the payload"
    );
    std::fs::write(&path, &tampered).unwrap();

    match inspect(&path, SUPPORTED_SCHEMA_VERSION) {
        SidecarState::Unreadable { reason } => assert!(reason.contains("checksum"), "{reason}"),
        other => panic!("a tampered document read as {other:?}"),
    }
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        tampered,
        "an unreadable sidecar is kept exactly as it was found"
    );
}

/// Each of §48.8.2's three `newer` causes refuses on its own. Decided before the checksum: a
/// newer writer's payload is not one this build can re-serialise to check.
#[test]
fn an_unknown_format_a_newer_schema_or_an_unknown_section_is_newer() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = written(dir.path());
    let base: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();

    let mut unknown_format = base.clone();
    unknown_format["format"] = serde_json::json!(3);
    let mut newer_schema = base.clone();
    newer_schema["schema_version"] = serde_json::json!(SUPPORTED_SCHEMA_VERSION + 1);
    let mut unknown_section = base;
    unknown_section["payload"]["sections"] =
        serde_json::json!({ "a_section_no_build_registers": [] });

    let mut newer = 0_u32;
    for (case, doc) in [
        ("unknown format", unknown_format),
        ("newer schema", newer_schema),
        ("unknown section", unknown_section),
    ] {
        std::fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
        match inspect(&path, SUPPORTED_SCHEMA_VERSION) {
            SidecarState::Newer { reason } => {
                eprintln!("{case}: {reason}");
                newer += 1;
            }
            other => eprintln!("{case} read as {other:?}"),
        }
    }
    eprintln!("documents refused as newer: {newer}");
    assert_eq!(newer, 3);
}

/// §48.8.2: the first corruption after the upgrade restores what format 1 held. The document
/// re-serialises to the bytes its checksum covers only while every field format 2 added stays
/// out of an empty record.
#[test]
fn a_format_1_document_reads_with_its_checksum_verified() {
    let doc = match inspect(&fixture(), SUPPORTED_SCHEMA_VERSION) {
        SidecarState::Present(doc) => doc,
        other => panic!("the format-1 fixture read as {other:?}"),
    };
    assert_eq!(doc.format, 1);
    assert_eq!(doc.generation, 5);
    assert_eq!(doc.schema_version, None);
    assert_eq!(doc.payload.projects.len(), 3);
    assert!(doc.payload.sections.is_empty());
    assert_eq!(doc.payload.pending, Vec::new());
    assert!(doc
        .payload
        .projects
        .iter()
        .all(|p| p.location_keys.is_empty() && p.removed_locations.is_empty()));
    let sessions: Vec<_> = doc
        .payload
        .projects
        .iter()
        .flat_map(|p| &p.sessions)
        .collect();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].segments.len(), 2);
    assert!(sessions[0].location_key.is_none());
}

/// Neither list travels in either direction: the export leaves them out, and the restore skips
/// them even when an older document carries them — so a key joining a list takes effect against
/// a document written before it joined.
#[test]
fn retired_and_rebuild_owned_keys_are_neither_exported_nor_restored() {
    let dropped: BTreeSet<&str> = REBUILD_OWNED_SETTINGS
        .iter()
        .chain(RETIRED_SETTINGS.iter())
        .copied()
        .collect();
    eprintln!("keys that never travel: {}", dropped.len());
    assert!(dropped.contains("sidecar_written_at"));
    assert!(dropped.contains("level_floor"));

    let dir = tempfile::tempdir().unwrap();
    let conn = migrated(dir.path());
    for key in &dropped {
        conn.execute(
            "INSERT INTO app_meta (k, v) VALUES (?1, '7')
             ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            [key],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO app_meta (k, v) VALUES ('first_run_completed_at', '9')",
        [],
    )
    .unwrap();
    let exported: BTreeSet<String> = export(&conn, 1, 1_000)
        .unwrap()
        .payload
        .settings
        .into_keys()
        .collect();
    assert_eq!(
        exported,
        BTreeSet::from(["first_run_completed_at".to_owned()])
    );

    let SidecarState::Present(doc) = inspect(&fixture(), SUPPORTED_SCHEMA_VERSION) else {
        panic!("the format-1 fixture did not read");
    };
    let carried: Vec<&String> = doc
        .payload
        .settings
        .keys()
        .filter(|k| dropped.contains(k.as_str()))
        .collect();
    eprintln!("dropped keys the format-1 document carries: {carried:?}");
    assert_eq!(carried.len(), 3, "the fixture must exercise the skip");

    // A fresh index seeds some of these keys itself; the restore must leave each as it was.
    let fresh = tempfile::tempdir().unwrap();
    let rebuilt = migrated(fresh.path());
    let before = app_meta(&rebuilt);
    restore_global(&rebuilt, &doc).unwrap();
    let after = app_meta(&rebuilt);
    for key in &dropped {
        assert_eq!(after.get(*key), before.get(*key), "{key} was restored");
    }
    assert!(after.contains_key("first_run_completed_at"));
    assert!(after.contains_key("wsl_consented_distros"));
}

/// The one dumper and the one loader round-trip every SQLite storage class through the
/// document's JSON, and the loader names only the columns the table has.
#[test]
fn a_dumped_row_loads_back_with_every_storage_class() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = migrated(dir.path());
    conn.execute_batch(
        "CREATE TABLE scratch (id INTEGER PRIMARY KEY, i INTEGER, r REAL, t TEXT, b BLOB, n TEXT);
         INSERT INTO scratch (i, r, t, b, n) VALUES (-7, 2.5, 'text', x'00ff10', NULL);",
    )
    .unwrap();
    let row = dump_row(&conn, "scratch", 1).unwrap();
    assert_eq!(row["b"], SidecarValue::Blob("00ff10".to_owned()));
    assert_eq!(row["r"], SidecarValue::Real(2.5));
    assert_eq!(row["n"], SidecarValue::Null);

    let mut carried: SidecarRow =
        serde_json::from_str(&serde_json::to_string(&row).unwrap()).unwrap();
    assert_eq!(carried, row);
    carried.insert("dropped_column".to_owned(), SidecarValue::Integer(1));
    let tx = conn.transaction().unwrap();
    insert_row(&tx, "scratch", &carried, &["id"]).unwrap();
    tx.commit().unwrap();

    let mut back = dump_row(&conn, "scratch", 2).unwrap();
    assert_eq!(back.remove("id"), Some(SidecarValue::Integer(2)));
    let mut original = row;
    original.remove("id");
    eprintln!("columns round-tripped: {}", back.len());
    assert_eq!(back.len(), 5);
    assert_eq!(back, original);
}

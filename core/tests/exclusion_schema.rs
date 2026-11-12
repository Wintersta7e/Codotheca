//! `0026_scan_exclusion.sql` — the user's exclusion list as a table, and `location.excluded`
//! (§56.11): the DDL's own refusals, the registration gate, and the sidecar section that carries
//! the list through a rebuild with its match key recomputed rather than trusted (§56.9).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;

use codotheca_core::index::path::{native_platform, PathPlatform};
use codotheca_core::index::rebuild::{rebuild_in_place, RebuildOutcome};
use codotheca_core::index::sidecar::unclassified;
use codotheca_core::index::Index;
use codotheca_core::paths::{path_from_bytes, path_key};
use codotheca_core::protocol::ExclusionKind;
use rusqlite::{params, Connection};

const NOW: i64 = 1_760_000_000;

/// The kind's stored word, read through the generated type's own serde name.
fn word(kind: ExclusionKind) -> String {
    match serde_json::to_value(kind).unwrap() {
        serde_json::Value::String(s) => s,
        other => panic!("{kind:?} serialised as {other}"),
    }
}

fn insert(
    conn: &Connection,
    kind: &str,
    entry: &str,
    path_bytes: Option<&[u8]>,
    match_key: &[u8],
    distro: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO scan_exclusion (kind, entry, path_bytes, match_key, distro, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![kind, entry, path_bytes, match_key, distro, NOW],
    )
}

fn fresh() -> (tempfile::TempDir, Index) {
    let dir = tempfile::tempdir().unwrap();
    let index = Index::open_at(dir.path(), NOW).unwrap();
    (dir, index)
}

#[test]
fn every_exclusion_kind_is_accepted_by_the_column() {
    let (_dir, index) = fresh();
    let conn = index.conn();
    let mut inserted = 0;
    for kind in ExclusionKind::ALL {
        let word = word(kind);
        let bytes = format!("/elsewhere/{word}").into_bytes();
        let path = (word == "path").then_some(bytes.as_slice());
        inserted += insert(conn, &word, &word, path, &bytes, "").unwrap();
    }
    eprintln!("inserted {inserted} of {} kinds", ExclusionKind::ALL.len());
    assert_eq!(inserted, ExclusionKind::ALL.len());
    assert_ne!(inserted, 0, "no kind was inserted");
}

#[test]
fn a_name_entry_carries_no_path_and_no_distro() {
    let (_dir, index) = fresh();
    let conn = index.conn();
    let with_path = insert(conn, "name", "scratch", Some(b"/x"), b"scratch", "");
    assert!(with_path.is_err(), "a name with path bytes was stored");
    let with_distro = insert(conn, "name", "scratch", None, b"scratch", "u");
    assert!(with_distro.is_err(), "a name with a distro was stored");
    let path_without_bytes = insert(conn, "path", "/x", None, b"/x", "");
    assert!(
        path_without_bytes.is_err(),
        "a path with no bytes was stored"
    );
    assert_eq!(
        insert(conn, "name", "scratch", None, b"scratch", "").unwrap(),
        1
    );
}

#[test]
fn a_byte_equal_duplicate_is_refused() {
    let (_dir, index) = fresh();
    let conn = index.conn();
    assert_eq!(
        insert(conn, "path", "/a", Some(b"/a"), b"/a", "d").unwrap(),
        1
    );
    assert!(insert(conn, "path", "/a again", Some(b"/a"), b"/a", "d").is_err());
    // The same key in another distro is another entry.
    assert_eq!(
        insert(conn, "path", "/a", Some(b"/a"), b"/a", "e").unwrap(),
        1
    );
}

#[test]
fn location_excluded_defaults_to_zero_and_holds_only_a_flag() {
    let (_dir, index) = fresh();
    let conn = index.conn();
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES ('p', 'p', 1, 1)",
        [],
    )
    .unwrap();
    let project = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind)
         VALUES (?1, 'linux', x'2f70', x'2f70', '/p', 's', 'present', 'worktree')",
        [project],
    )
    .unwrap();
    let location = conn.last_insert_rowid();
    let excluded: i64 = conn
        .query_row(
            "SELECT excluded FROM location WHERE id = ?1",
            [location],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(excluded, 0);
    assert!(conn
        .execute("UPDATE location SET excluded = 2 WHERE id = ?1", [location])
        .is_err());
    assert_eq!(
        conn.execute("UPDATE location SET excluded = 1 WHERE id = ?1", [location])
            .unwrap(),
        1
    );
}

#[test]
fn the_registration_gate_classifies_the_table_and_the_column() {
    let (_dir, index) = fresh();
    match unclassified(index.conn()) {
        Ok(report) => eprintln!(
            "classified {} tables, {} columns",
            report.tables, report.columns
        ),
        Err(findings) => panic!("unclassified: {findings:#?}"),
    }
}

/// The key a restored path row must hold: `path_key` of its bytes under its own platform.
fn expected_key(bytes: &[u8], platform: PathPlatform) -> Vec<u8> {
    path_key(&path_from_bytes(bytes), platform)
}

/// `(kind, entry, path_bytes, match_key, distro, created_at)`.
type Row = (String, String, Option<Vec<u8>>, Vec<u8>, String, i64);

fn stored(conn: &Connection) -> Vec<Row> {
    conn.prepare(
        "SELECT kind, entry, path_bytes, match_key, distro, created_at FROM scan_exclusion
         ORDER BY created_at, id",
    )
    .unwrap()
    .query_map([], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
        ))
    })
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

fn rebuild(dir: &Path, index: Index) -> Index {
    index.export_sidecar(NOW).unwrap();
    drop(index);
    std::fs::write(Index::db_path(dir), b"this is not a database").unwrap();
    match rebuild_in_place(dir, NOW + 1) {
        Ok(RebuildOutcome::Rebuilt(_)) => {}
        other => panic!("expected a rebuild, got {other:?}"),
    }
    Index::open_at(dir, NOW + 2).unwrap()
}

/// §56.9: the export never carries `match_key`, so a key that was wrong in the old index comes
/// back as the key of the bytes. Every row's other columns come back as they were.
#[test]
fn scan_exclusions_round_trip_recomputes_match_key() {
    let (dir, index) = fresh();
    let conn = index.conn();
    let native = b"/elsewhere/Private".as_slice();
    let in_distro = b"/home/u/Scratch".as_slice();
    conn.execute(
        "INSERT INTO scan_exclusion (kind, entry, path_bytes, match_key, distro, created_at)
         VALUES ('name', 'scratch', NULL, CAST('scratch' AS BLOB), '', 10),
                ('path', '/elsewhere/Private', ?1, CAST('tampered' AS BLOB), '', 20),
                ('path', '/home/u/Scratch', ?2, CAST('also tampered' AS BLOB), 'd', 30)",
        params![native, in_distro],
    )
    .unwrap();
    let before = stored(conn);

    let rebuilt = rebuild(dir.path(), index);
    let after = stored(rebuilt.conn());
    eprintln!("restored {} of {} exclusions", after.len(), before.len());
    assert_eq!(after.len(), before.len());
    for (was, now) in before.iter().zip(&after) {
        assert_eq!(
            (&was.0, &was.1, &was.2, &was.4, was.5),
            (&now.0, &now.1, &now.2, &now.4, now.5)
        );
    }
    assert_eq!(
        after[0].3, b"scratch",
        "a name's key is its own bytes, unfolded"
    );
    assert_eq!(after[1].3, expected_key(native, native_platform()));
    assert_eq!(after[2].3, expected_key(in_distro, PathPlatform::Unix));
}

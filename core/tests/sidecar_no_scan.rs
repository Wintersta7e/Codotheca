//! §48.8.3's `no_scan` mode: a project whose every copy this app removed is one no scan will ever
//! find again, so the rebuild re-creates it — every column, its ids kept — and applies its
//! pending records in the same transaction.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;
use std::path::Path;

use codotheca_core::index::noscan::is_no_scan;
use codotheca_core::index::rebuild::{rebuild_in_place, RebuildOutcome, RebuildReportFile};
use codotheca_core::index::subject::subject_for_project;
use codotheca_core::index::Index;
use codotheca_core::protocol::ProjectId;
use rusqlite::{params, Connection};

const NOW: i64 = 1_760_000_000;

/// Every column of one row, as text, so two rows compare column by column.
fn dump(conn: &Connection, table: &str, id: i64) -> BTreeMap<String, String> {
    let mut stmt = conn
        .prepare(&format!("SELECT * FROM {table} WHERE id = ?1"))
        .unwrap();
    let names: Vec<String> = stmt.column_names().into_iter().map(str::to_owned).collect();
    stmt.query_row([id], |r| {
        names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let value: rusqlite::types::Value = r.get(i)?;
                Ok((name.clone(), format!("{value:?}")))
            })
            .collect::<rusqlite::Result<_>>()
    })
    .unwrap()
}

/// A project uninstalled from both its copies, at the ids given: a note, odd columns set, one
/// session of one segment in the first copy, and one session-track XP row.
fn uninstalled(conn: &Connection, id: i64, first_location: i64) {
    conn.execute(
        "INSERT INTO project (id, name, seed_basename, lineage_key, remote_key, notes,
                              is_archived, primary_language, description, art_state,
                              reroll_offset, last_commit_at, created_at, updated_at)
         VALUES (?1, 'gone', 'gone-dir', 'gone-lineage', 'example.invalid/owner/gone',
                 'a kept note', 1, 'Rust', 'what it was', 'ready', 3, 777, 11, 22)",
        [id],
    )
    .unwrap();
    for (offset, path) in [(0_i64, "/r/gone-a"), (1, "/r/gone-b")] {
        conn.execute(
            "INSERT INTO location (id, project_id, kind, path_bytes, path_key, path_display,
                                   store_key, presence, repo_kind, branch, removed_at)
             VALUES (?1, ?2, 'linux', ?3, ?3, ?4, 's', 'missing', 'worktree', 'main', ?5)",
            params![
                first_location + offset,
                id,
                path.as_bytes(),
                path,
                900 + offset
            ],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO session (project_id, location_id, started_at, ended_at, credited_seconds,
                              close_reason)
         VALUES (?1, ?2, 100, 160, 60, 'stop')",
        [id, first_location],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO session_segment (session_id, started_at, ended_at, credited_seconds,
                                      closed_by)
         VALUES (?1, 100, 160, 60, 'session_end')",
        [conn.last_insert_rowid()],
    )
    .unwrap();
    let subject = subject_for_project(conn, ProjectId(id))
        .unwrap()
        .unwrap()
        .to_key();
    conn.execute(
        "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                track, meta)
         VALUES (100, 0, ?1, ?2, 'session', ?3, 'session', NULL)",
        params![id, subject, format!("session:{subject}:0")],
    )
    .unwrap();
}

/// A live project with one present copy, which only a scan brings back.
fn live(conn: &Connection) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
         VALUES ('here', 'here', 'here-lineage', 1, 1)",
        [],
    )
    .unwrap();
    let id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind)
         VALUES (?1, 'linux', x'2f722f68657265', x'2f722f68657265', '/r/here', 's', 'present',
                 'worktree')",
        [id],
    )
    .unwrap();
    id
}

/// Export, close, overwrite the index with bytes SQLite reads as no database, and rebuild.
fn export_corrupt_and_rebuild(dir: &Path, index: Index) -> RebuildReportFile {
    index.export_sidecar(NOW).unwrap();
    drop(index);
    std::fs::write(Index::db_path(dir), b"this is not a database").unwrap();
    match rebuild_in_place(dir, NOW + 1) {
        Ok(RebuildOutcome::Rebuilt(report)) => report,
        other => panic!("expected a rebuild, got {other:?}"),
    }
}

fn count(conn: &Connection, sql: &str, id: i64) -> i64 {
    conn.query_row(sql, [id], |r| r.get(0)).unwrap()
}

/// AC-P4-48-21: an uninstalled project — every copy removed — comes back from the rebuild alone,
/// with no hand-off and no scan: every `project` and `location` column as exported, its ids kept,
/// its session restored once through the pending matcher, and none of its records left pending.
#[test]
fn ac_p4_48_21_no_scan_projects_are_recreated_without_a_scan() {
    let dir = tempfile::tempdir().unwrap();
    let original = Index::open_at(dir.path(), NOW).unwrap();
    let conn = original.conn();
    let here = live(conn);
    uninstalled(conn, 5, 8);
    let gone_subject = subject_for_project(conn, ProjectId(5))
        .unwrap()
        .unwrap()
        .to_key();
    let before: Vec<BTreeMap<String, String>> = vec![
        dump(conn, "project", 5),
        dump(conn, "location", 8),
        dump(conn, "location", 9),
    ];

    let report = export_corrupt_and_rebuild(dir.path(), original);
    eprintln!(
        "restored: {:?}, pending: {}",
        report.restored, report.pending
    );
    assert_eq!(report.restored.get("no_scan_projects"), Some(&1));

    let rebuilt = Index::open_at(dir.path(), NOW + 2).unwrap();
    let back = rebuilt.conn();
    let after = [
        dump(back, "project", 5),
        dump(back, "location", 8),
        dump(back, "location", 9),
    ];
    let mut compared = 0_u32;
    for (old, new) in before.iter().zip(&after) {
        for (column, value) in old {
            assert_eq!(&new[column], value, "column {column}");
            compared += 1;
        }
    }
    eprintln!("project and location columns compared: {compared}");
    assert_ne!(compared, 0);

    assert_eq!(
        count(
            back,
            "SELECT count(*) FROM session WHERE project_id = ?1",
            5
        ),
        1,
        "the session is restored once"
    );
    assert_eq!(
        count(
            back,
            "SELECT count(*) FROM session WHERE project_id = 5 AND location_id = ?1",
            8
        ),
        1,
        "the session names the copy it ran in"
    );
    assert_eq!(
        count(
            back,
            "SELECT count(*) FROM xp_events WHERE project_id = ?1 AND track = 'session'",
            5
        ),
        1
    );
    let waiting: Vec<String> = back
        .prepare("SELECT subject_key FROM sidecar_pending ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    eprintln!("pending after the rebuild: {waiting:?}");
    assert!(
        !waiting.contains(&gone_subject),
        "a record of the re-created project waits for a scan that never comes"
    );
    assert_eq!(
        count(back, "SELECT count(*) FROM project WHERE id = ?1", here),
        0,
        "a live project waits for the scan"
    );
    assert_eq!(waiting.len(), 1, "the live project's record waits");
    assert_eq!(report.pending, 1);
}

/// The restore raises each table's `AUTOINCREMENT` sequence past the ids it kept, so no row the
/// scan creates later takes an id a removed project held — even once that project is gone.
#[test]
fn a_scan_created_project_never_reuses_a_restored_id() {
    let dir = tempfile::tempdir().unwrap();
    let original = Index::open_at(dir.path(), NOW).unwrap();
    uninstalled(original.conn(), 40, 70);
    export_corrupt_and_rebuild(dir.path(), original);

    let rebuilt = Index::open_at(dir.path(), NOW + 2).unwrap();
    let conn = rebuilt.conn();
    let seq = |table: &str| -> i64 {
        conn.query_row(
            "SELECT seq FROM sqlite_sequence WHERE name = ?1",
            [table],
            |r| r.get(0),
        )
        .unwrap()
    };
    eprintln!(
        "sequences: project {}, location {}",
        seq("project"),
        seq("location")
    );
    assert!(seq("project") >= 40);
    assert!(seq("location") >= 71);

    // Gone again — the sequence, not the surviving rows, is what keeps the ids unique.
    conn.execute_batch(
        "DELETE FROM xp_events; DELETE FROM session_segment; DELETE FROM session;
         DELETE FROM location; DELETE FROM project;",
    )
    .unwrap();
    let project = live(conn);
    let location: i64 = conn
        .query_row(
            "SELECT id FROM location WHERE project_id = ?1",
            [project],
            |r| r.get(0),
        )
        .unwrap();
    eprintln!("a scan-created project: {project}, its copy: {location}");
    assert!(project > 40, "project id {project} reuses a restored one");
    assert!(
        location > 71,
        "location id {location} reuses a restored one"
    );
}

/// No scan can find a project whose every copy is removed; one with no copy at all is a project
/// not cloned yet, which has no subject and is listed again by sync, so it is not one.
#[test]
fn a_zero_location_project_is_not_no_scan() {
    let dir = tempfile::tempdir().unwrap();
    let index = Index::open_at(dir.path(), NOW).unwrap();
    let conn = index.conn();
    let mut cases = 0_u32;

    conn.execute(
        "INSERT INTO project (name, seed_basename, remote_key, created_at, updated_at)
         VALUES ('listed', 'listed', 'example.invalid/owner/listed', 1, 1)",
        [],
    )
    .unwrap();
    let listed = ProjectId(conn.last_insert_rowid());
    let no_location = is_no_scan(conn, listed).unwrap();
    eprintln!("no location: {no_location}");
    assert!(!no_location, "a not-cloned project is not no_scan");
    cases += 1;

    uninstalled(conn, 20, 30);
    let all_removed = is_no_scan(conn, ProjectId(20)).unwrap();
    eprintln!("every location removed: {all_removed}");
    assert!(all_removed);
    cases += 1;

    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind)
         VALUES (20, 'linux', x'2f722f6261636b', x'2f722f6261636b', '/r/back', 's', 'present',
                 'worktree')",
        [],
    )
    .unwrap();
    let one_live = is_no_scan(conn, ProjectId(20)).unwrap();
    eprintln!("one location live again: {one_live}");
    assert!(!one_live, "a project with a live copy is the scan's");
    cases += 1;

    let here = ProjectId(live(conn));
    let live_project = is_no_scan(conn, here).unwrap();
    eprintln!("a live project: {live_project}");
    assert!(!live_project);
    cases += 1;

    eprintln!("no_scan cases: {cases}");
    assert_eq!(cases, 4);
}

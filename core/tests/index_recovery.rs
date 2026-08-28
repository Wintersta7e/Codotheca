#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::io::Write as _;

use codotheca_core::index::recovery::{quarantine, QuarantinedFiles};
use codotheca_core::index::{open_connection, Index, IndexError};

#[test]
fn an_orphaned_wal_is_recovered_and_is_not_corruption() {
    let live = tempfile::tempdir().unwrap();
    let db = Index::db_path(live.path());
    let conn = open_connection(&db).unwrap();
    conn.execute_batch("CREATE TABLE t (v TEXT); INSERT INTO t VALUES ('survived');")
        .unwrap();

    // Simulate a crash: take the files while the process still holds them, so the copy has a
    // WAL that was never checkpointed and no owner.
    let crashed = tempfile::tempdir().unwrap();
    for suffix in ["", "-wal"] {
        let mut from = db.as_os_str().to_os_string();
        from.push(suffix);
        let from = std::path::PathBuf::from(from);
        if from.exists() {
            let mut to = Index::db_path(crashed.path()).as_os_str().to_os_string();
            to.push(suffix);
            std::fs::copy(&from, std::path::PathBuf::from(to)).unwrap();
        }
    }
    drop(conn);

    assert!(
        crashed.path().join("index.db-wal").exists(),
        "the fixture must actually have an orphaned WAL or it tests nothing"
    );

    let recovered = open_connection(&Index::db_path(crashed.path())).unwrap();
    let v: String = recovered
        .query_row("SELECT v FROM t", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, "survived");
}

#[test]
fn a_file_that_is_not_a_database_is_reported_as_corrupt() {
    let dir = tempfile::tempdir().unwrap();
    let db = Index::db_path(dir.path());
    std::fs::create_dir_all(dir.path()).unwrap();
    let mut f = std::fs::File::create(&db).unwrap();
    f.write_all(b"this is not a database, it is 42 bytes of text")
        .unwrap();
    drop(f);

    match open_connection(&db) {
        Err(IndexError::Corrupt { detail }) => assert!(!detail.is_empty()),
        other => panic!("expected Corrupt, got {other:?}"),
    }
}

#[test]
fn quarantine_moves_the_database_wal_and_shm_together() {
    let dir = tempfile::tempdir().unwrap();
    let db = Index::db_path(dir.path());
    std::fs::create_dir_all(dir.path()).unwrap();
    for suffix in ["", "-wal", "-shm"] {
        let mut p = db.as_os_str().to_os_string();
        p.push(suffix);
        std::fs::write(std::path::PathBuf::from(p), b"x").unwrap();
    }

    let QuarantinedFiles {
        db: moved_db,
        wal,
        shm,
        at,
    } = quarantine(&db, 1_787_126_520).unwrap();
    assert_eq!(at, 1_787_126_520);
    assert!(
        !db.exists(),
        "the corrupt database must not be left in place"
    );
    assert!(moved_db.exists());
    assert_eq!(
        moved_db.file_name().unwrap().to_string_lossy(),
        "index.db.corrupt-1787126520"
    );
    assert!(
        wal.unwrap().exists(),
        "a stale WAL left behind would be replayed into the rebuild"
    );
    assert!(shm.unwrap().exists());
}

#[test]
fn quarantine_tolerates_a_database_with_no_wal_or_shm() {
    let dir = tempfile::tempdir().unwrap();
    let db = Index::db_path(dir.path());
    std::fs::create_dir_all(dir.path()).unwrap();
    std::fs::write(&db, b"x").unwrap();

    let q = quarantine(&db, 7).unwrap();
    assert!(q.wal.is_none());
    assert!(q.shm.is_none());
}

use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::sidecar::{
    export, restore_for_subject, restore_global, write_atomically, RestoreCounts, SidecarCounts,
};
use codotheca_core::index::subject::{resolve_subject, ProjectSubject};
use codotheca_core::protocol::ProjectId;

fn seed_and_export(dir: &std::path::Path) {
    let mut conn = open_connection(&Index::db_path(dir)).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    conn.execute_batch(
        "INSERT INTO project (name, seed_basename, lineage_key, remote_key, notes,
                              is_pinned, reroll_offset, created_at, updated_at)
         VALUES ('thing', 'thing-dir', 'lineage-1', 'h/o/n', 'a note', 1, 4, 1, 1);
         INSERT INTO session (project_id, started_at, ended_at, credited_seconds, close_reason)
         VALUES (1, 100, 200, 90, 'stop');
         INSERT INTO session_segment (session_id, started_at, ended_at, credited_seconds, closed_by)
         VALUES (1, 100, 190, 90, 'session_end');
         INSERT INTO scan_root (kind, path_bytes, path_key, path_display, added_by, added_at)
         VALUES ('linux', x'2f686f6d65', x'2f686f6d65', '/home', 'user', 5);
         INSERT INTO identity (email, source, confirmed_at)
         VALUES ('a@example.invalid', 'gitconfig', 42);
         INSERT INTO collection (name, kind) VALUES ('By hand', 'manual');
         INSERT INTO collection_member (collection_id, project_id) VALUES (1, 1);
         INSERT INTO app_meta (k, v) VALUES ('effects_tier', 'reduced');
         INSERT INTO view_state (k, v) VALUES ('shelf.sort', 'touched');",
    )
    .unwrap();
    let doc = export(&conn, 3, 900).unwrap();
    write_atomically(&doc, &Index::sidecar_path(dir)).unwrap();
}

#[test]
fn rebuild_quarantines_restores_the_global_half_and_dates_the_gap() {
    let dir = tempfile::tempdir().unwrap();
    seed_and_export(dir.path());

    // Corrupt the database under the sidecar.
    std::fs::write(Index::db_path(dir.path()), b"not a database at all").unwrap();

    let (index, report) = Index::rebuild(dir.path(), 5_000).unwrap();
    assert!(report.quarantined.db.exists());

    assert_eq!(report.restored.roots, 1);
    assert_eq!(report.restored.identities, 1);
    assert_eq!(report.restored.collections, 1);
    assert_eq!(report.restored.view_state, 1);
    assert!(report.restored.settings >= 1);
    assert_eq!(
        report.restored.projects, 0,
        "a rebuilt database has no projects yet; those records wait for the scan"
    );

    assert_eq!(report.deferred.projects, 1);
    assert_eq!(report.deferred.sessions, 1);
    assert_eq!(report.deferred.notes, 1);

    assert_eq!(report.gap_started_at, Some(900));
    assert!(
        !report.gap_counts_recoverable,
        "the counts were in the database that was destroyed; a 0 here would be a lie"
    );

    let roots: i64 = index
        .conn()
        .query_row("SELECT count(*) FROM scan_root", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        roots, 1,
        "the user is not asked for consent and roots again"
    );
    let tier: String = index
        .conn()
        .query_row("SELECT v FROM app_meta WHERE k='effects_tier'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(tier, "reduced");
}

#[test]
fn the_project_half_restores_when_the_scan_rediscovers_the_subject() {
    let dir = tempfile::tempdir().unwrap();
    seed_and_export(dir.path());
    let doc = codotheca_core::index::sidecar::read(&Index::sidecar_path(dir.path())).unwrap();

    std::fs::write(Index::db_path(dir.path()), b"not a database at all").unwrap();
    let (index, _) = Index::rebuild(dir.path(), 5_000).unwrap();

    // The scan re-derives the project, and gets a different id than it had before.
    index
        .conn()
        .execute_batch(
            "INSERT INTO project (id, name, seed_basename, lineage_key, remote_key,
                                  created_at, updated_at)
             VALUES (77, 'thing', 'thing-dir', 'lineage-1', 'h/o/n', 5000, 5000)",
        )
        .unwrap();
    let subject = ProjectSubject::Lineage {
        lineage_key: "lineage-1".into(),
        remote_key: Some("h/o/n".into()),
    };
    let id = resolve_subject(index.conn(), &subject).unwrap().unwrap();
    assert_eq!(id, ProjectId(77));

    let counts = restore_for_subject(index.conn(), &doc, id).unwrap();
    assert_eq!(counts.projects, 1);
    assert_eq!(counts.notes, 1);
    assert_eq!(counts.sessions, 1);
    assert_eq!(counts.session_segments, 1);
    assert_eq!(counts.collection_members, 1);

    let (notes, pinned, reroll): (String, i64, i64) = index
        .conn()
        .query_row(
            "SELECT notes, is_pinned, reroll_offset FROM project WHERE id=77",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(notes, "a note");
    assert_eq!(pinned, 1);
    assert_eq!(
        reroll, 4,
        "§7.2: the reroll offset is not derivable and must come back"
    );
}

#[test]
fn a_rebuild_with_no_sidecar_reports_no_gap_and_restores_nothing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path()).unwrap();
    std::fs::write(Index::db_path(dir.path()), b"not a database at all").unwrap();

    let (_index, report) = Index::rebuild(dir.path(), 5_000).unwrap();
    assert_eq!(report.gap_started_at, None);
    assert_eq!(report.restored, RestoreCounts::default());
    assert_eq!(report.deferred, SidecarCounts::default());
}

#[test]
fn restore_global_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    seed_and_export(dir.path());
    let doc = codotheca_core::index::sidecar::read(&Index::sidecar_path(dir.path())).unwrap();

    let target = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(target.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();

    restore_global(&conn, &doc).unwrap();
    restore_global(&conn, &doc).unwrap();
    let roots: i64 = conn
        .query_row("SELECT count(*) FROM scan_root", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        roots, 1,
        "a second restore must not double the user's roots"
    );
}

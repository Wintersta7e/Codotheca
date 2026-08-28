#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::sidecar::{
    counts, export, is_due, read, write_atomically, SIDECAR_FORMAT, SIDECAR_INTERVAL_SECS,
};
use codotheca_core::index::{open_connection, Index};

fn seeded() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    conn.execute_batch(
        "INSERT INTO project (name, seed_basename, lineage_key, remote_key, notes,
                              is_pinned, reroll_offset, created_at, updated_at)
         VALUES ('thing', 'thing-dir', 'lineage-1', 'h/o/n', 'a note', 1, 3, 1, 1);

         INSERT INTO session (project_id, started_at, ended_at, credited_seconds, close_reason)
         VALUES (1, 100, 200, 90, 'stop');
         INSERT INTO session_segment (session_id, started_at, ended_at, credited_seconds, closed_by)
         VALUES (1, 100, 190, 90, 'session_end');

         INSERT INTO xp_events (ts, project_id, subject_key, kind, dedupe_key, track)
         VALUES (150, 1, 'lineage:lineage-1|remote:h/o/n', 'session', 'session:1', 'session');
         INSERT INTO xp_events (ts, project_id, subject_key, kind, dedupe_key, track)
         VALUES (160, 1, 'lineage:lineage-1|remote:h/o/n', 'commit_day',
                 'commit_day:lineage-1:h/o/n:2026-08-21', 'git');

         INSERT INTO launch_target (project_id, kind, name, exec_bytes, sort_index, detected)
         VALUES (1, 'editor', 'mine', x'2f6100ff', 0, 0);
         INSERT INTO launch_target (project_id, kind, name, exec_bytes, sort_index, detected)
         VALUES (1, 'editor', 'found', x'2f62', 1, 1);

         INSERT INTO collection (name, kind) VALUES ('By hand', 'manual');
         INSERT INTO collection_member (collection_id, project_id) VALUES (1, 1);

         INSERT INTO scan_root (kind, path_bytes, path_key, path_display, added_by, added_at)
         VALUES ('linux', x'2f686f6d65', x'2f686f6d65', '/home', 'user', 5);

         INSERT INTO identity (email, name, source, confirmed_at)
         VALUES ('a@example.invalid', 'A', 'gitconfig', 42);
         INSERT INTO identity_alias (identity_id, email, reason)
         VALUES (1, 'a+x@example.invalid', 'local_part');

         INSERT INTO app_meta (k, v) VALUES ('effects_tier', 'reduced');
         INSERT INTO app_meta (k, v) VALUES ('git_version', '2.45.0');
         INSERT INTO view_state (k, v) VALUES ('shelf.sort', 'touched');",
    )
    .unwrap();
    (dir, conn)
}

#[test]
fn the_export_is_keyed_on_a_subject_and_not_on_an_id() {
    let (_d, conn) = seeded();
    let s = export(&conn, 1, 1_000).unwrap();
    assert_eq!(s.format, SIDECAR_FORMAT);
    assert_eq!(s.generation, 1);
    assert_eq!(s.written_at, 1_000);
    assert_eq!(s.payload.projects.len(), 1);

    let p = &s.payload.projects[0];
    assert_eq!(p.subject, "lineage:lineage-1|remote:h/o/n");
    assert_eq!(p.notes.as_deref(), Some("a note"));
    assert!(p.is_pinned);
    assert_eq!(p.reroll_offset, 3);
    assert_eq!(p.seed_basename, "thing-dir");

    let json = serde_json::to_string(&s).unwrap();
    assert!(
        !json.contains("\"project_id\""),
        "an id in the export is what a rebuild invalidates"
    );
}

#[test]
fn only_the_session_track_and_only_custom_targets_are_exported() {
    let (_d, conn) = seeded();
    let s = export(&conn, 1, 1_000).unwrap();
    let p = &s.payload.projects[0];

    assert_eq!(
        p.xp_events.len(),
        1,
        "git-derived rows recompute from history"
    );
    assert_eq!(p.xp_events[0].kind, "session");

    assert_eq!(p.launch_targets.len(), 1, "detected rows are re-detected");
    assert_eq!(p.launch_targets[0].name, "mine");
    assert_eq!(
        p.launch_targets[0].exec_hex, "2f6100ff",
        "an executable path is bytes, and 0xff is not UTF-8"
    );

    assert_eq!(p.sessions.len(), 1);
    assert_eq!(p.sessions[0].credited_seconds, 90);
    assert_eq!(p.sessions[0].segments.len(), 1);
}

#[test]
fn the_rebuild_owned_settings_keys_are_not_exported() {
    let (_d, conn) = seeded();
    let s = export(&conn, 1, 1_000).unwrap();
    assert_eq!(
        s.payload.settings.get("effects_tier").map(String::as_str),
        Some("reduced")
    );
    for owned in ["schema_version", "sidecar_generation", "git_version"] {
        assert!(
            !s.payload.settings.contains_key(owned),
            "{owned} belongs to the new database, not to the export"
        );
    }
    assert_eq!(
        s.payload.view_state.get("shelf.sort").map(String::as_str),
        Some("touched")
    );
}

#[test]
fn the_rest_of_the_non_derivable_set_is_present() {
    let (_d, conn) = seeded();
    let s = export(&conn, 1, 1_000).unwrap();
    assert_eq!(s.payload.collections.len(), 1);
    assert_eq!(
        s.payload.collections[0].members,
        vec!["lineage:lineage-1|remote:h/o/n"]
    );
    assert_eq!(s.payload.roots.len(), 1);
    assert_eq!(s.payload.roots[0].path_hex, "2f686f6d65");
    assert_eq!(s.payload.identities.len(), 1);
    assert_eq!(s.payload.identities[0].confirmed_at, Some(42));
    assert_eq!(s.payload.identities[0].aliases.len(), 1);

    let c = counts(&s);
    assert_eq!(c.projects, 1);
    assert_eq!(c.notes, 1);
    assert_eq!(c.sessions, 1);
    assert_eq!(c.session_segments, 1);
    assert_eq!(c.xp_events, 1);
    assert_eq!(c.launch_targets, 1);
    assert_eq!(c.collections, 1);
    assert_eq!(c.collection_members, 1);
    assert_eq!(c.roots, 1);
    assert_eq!(c.identities, 1);
}

#[test]
fn the_file_is_written_atomically_and_read_back_verified() {
    let (dir, conn) = seeded();
    let path = Index::sidecar_path(dir.path());
    let s = export(&conn, 7, 1_000).unwrap();
    write_atomically(&s, &path).unwrap();

    assert!(path.exists());
    assert!(
        std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .all(|e| !e.file_name().to_string_lossy().ends_with(".tmp")),
        "the temporary file must not be left behind"
    );

    let back = read(&path).unwrap();
    assert_eq!(back, s);
    assert_eq!(back.generation, 7);
}

#[test]
fn a_tampered_sidecar_is_refused_rather_than_restored() {
    let (dir, conn) = seeded();
    let path = Index::sidecar_path(dir.path());
    write_atomically(&export(&conn, 1, 1_000).unwrap(), &path).unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    let tampered = text.replace("a note", "a NOTE");
    assert_ne!(
        tampered, text,
        "the fixture must actually change the payload"
    );
    std::fs::write(&path, tampered).unwrap();

    assert!(
        read(&path).is_err(),
        "the checksum exists to catch exactly this"
    );
}

#[test]
fn due_is_hourly_and_a_never_written_sidecar_is_due_now() {
    assert!(is_due(None, 0));
    assert!(!is_due(Some(1_000), 1_000 + SIDECAR_INTERVAL_SECS - 1));
    assert!(is_due(Some(1_000), 1_000 + SIDECAR_INTERVAL_SECS));
}

#[test]
fn export_sidecar_bumps_the_generation_and_records_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = codotheca_core::index::Index::open(dir.path()).unwrap();
    index
        .conn_mut()
        .execute(
            "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
             VALUES ('thing', 'thing', 'l1', 1, 1)",
            [],
        )
        .unwrap();

    let first = index.export_sidecar(1_000).unwrap();
    assert_eq!(first.generation, 1);
    assert_eq!(first.written_at, 1_000);
    assert!(!index.sidecar_due(1_000).unwrap());

    let second = index.export_sidecar(1_000 + SIDECAR_INTERVAL_SECS).unwrap();
    assert_eq!(second.generation, 2);
    assert_eq!(read(&first.path).unwrap().generation, 2);
}

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
#![cfg(feature = "testkit")]
//! Launch-target detection: the global rows it writes, and a re-detection that rewrites them.

use codotheca_core::index::Index;
use codotheca_core::launch::catalogue::TargetKind;
use codotheca_core::launch::detect::{detect_and_store, rewrite_exec_for};
use codotheca_core::launch::probe::{FakeProbe, ProbeSource};

fn index() -> (tempfile::TempDir, Index) {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    (dir, index)
}

fn rows(conn: &rusqlite::Connection) -> Vec<(i64, String, String, i64, Vec<u8>)> {
    let mut stmt = conn
        .prepare(
            "SELECT id, kind, name, sort_index, exec_bytes FROM launch_target ORDER BY kind, sort_index",
        )
        .expect("prepare");
    let mapped = stmt
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .expect("query");
    mapped.map(|r| r.expect("row")).collect()
}

#[test]
fn detection_writes_only_the_global_scope_and_never_a_language_row() {
    let (dir, mut index) = index();
    let mut probe = FakeProbe::new();
    probe
        .app("/usr/bin/code", ProbeSource::Desktop)
        .app("/usr/bin/kitty", ProbeSource::Path);
    let report =
        detect_and_store(index.conn_mut(), &probe, dir.path(), "/home/u", 1_000).expect("detect");
    assert!(report.inserted >= 2);

    let scoped: i64 = index
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM launch_target
             WHERE project_id IS NOT NULL OR location_id IS NOT NULL OR language IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(scoped, 0, "detection wrote a scoped row");
}

#[test]
fn the_editor_with_the_most_overlapping_recents_heads_the_editor_scope() {
    let (dir, mut index) = index();
    // Two indexed locations, and a recents file naming both of them.
    index
        .conn()
        .execute_batch(
            "INSERT INTO project (id, name, seed_basename, created_at, updated_at)
                 VALUES (1, 'p', 'p', 0, 0);
             INSERT INTO location (id, project_id, kind, distro, path_bytes, path_key, path_display,
                                   volume_key, store_key, presence, repo_kind)
                 VALUES (1, 1, 'linux', '', X'2F636F64652F6F6E65', X'2F636F64652F6F6E65',
                         '/code/one', 'v', 's', 'present', 'worktree');",
        )
        .expect("seed");

    let config = dir.path().join("cfg");
    std::fs::create_dir_all(config.join("options")).expect("mkdir");
    std::fs::write(
        config.join("options/recentProjects.xml"),
        r#"<map><entry key="/code/one" /></map>"#,
    )
    .expect("write");

    let mut probe = FakeProbe::new();
    probe
        .app("/usr/bin/idea", ProbeSource::Desktop)
        .app("/usr/bin/zed", ProbeSource::Desktop);
    let report =
        detect_and_store(index.conn_mut(), &probe, &config, "/home/u", 1_000).expect("detect");

    let head: String = index
        .conn()
        .query_row(
            "SELECT name FROM launch_target
             WHERE kind = 'editor' AND project_id IS NULL AND location_id IS NULL AND language IS NULL
             ORDER BY sort_index LIMIT 1",
            [],
            |r| r.get(0),
        )
        .expect("head");
    assert_eq!(head, "JetBrains IDE");
    assert!(report.default_target_id.is_some());
}

#[test]
fn re_detection_rewrites_every_row_of_that_kind_and_name_including_scoped_ones() {
    let (dir, mut index) = index();
    let mut probe = FakeProbe::new();
    probe.app("/apps/app-1.0/code", ProbeSource::Desktop);
    detect_and_store(index.conn_mut(), &probe, dir.path(), "/home/u", 1_000).expect("detect");

    // A per-project override copied from the global row, as targets.setDefault writes one.
    index
        .conn()
        .execute_batch(
            "INSERT INTO project (id, name, seed_basename, created_at, updated_at)
                 VALUES (1, 'p', 'p', 0, 0);
             INSERT INTO launch_target
                 (project_id, location_id, language, kind, name, exec_bytes, args_json, cwd_mode,
                  env_json, sort_index, detected, verify_state)
             SELECT 1, NULL, NULL, kind, name, exec_bytes, args_json, cwd_mode, env_json, 0, 1, 'ok'
             FROM launch_target WHERE kind = 'editor';
             INSERT INTO launch_target
                 (project_id, location_id, language, kind, name, exec_bytes, args_json, cwd_mode,
                  env_json, sort_index, detected, verify_state)
             SELECT NULL, NULL, 'Rust', kind, name, exec_bytes, args_json, cwd_mode, env_json, 0, 1, 'ok'
             FROM launch_target WHERE kind = 'editor' AND project_id IS NULL AND language IS NULL;",
        )
        .expect("seed overrides");

    let tx = index.conn_mut().transaction().expect("tx");
    let n = rewrite_exec_for(
        &tx,
        TargetKind::Editor,
        "Code",
        b"/apps/app-2.0/code",
        2_000,
    )
    .expect("rewrite");
    tx.commit().expect("commit");
    assert_eq!(
        n, 3,
        "the global row, the project override and the language row"
    );

    let stranded: i64 = index
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM launch_target
              WHERE exec_bytes = X'2F617070732F6170702D312E302F636F6465'",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(
        stranded, 0,
        "a row was left pointing into the old versioned directory"
    );
}

#[test]
fn detection_is_idempotent_and_does_not_multiply_rows() {
    let (dir, mut index) = index();
    let mut probe = FakeProbe::new();
    probe.app("/usr/bin/code", ProbeSource::Desktop);
    detect_and_store(index.conn_mut(), &probe, dir.path(), "/home/u", 1_000).expect("first");
    let before = rows(index.conn()).len();
    detect_and_store(index.conn_mut(), &probe, dir.path(), "/home/u", 2_000).expect("second");
    assert_eq!(rows(index.conn()).len(), before);
}

#[test]
fn nothing_detected_writes_nothing_rather_than_a_placeholder_row() {
    let (dir, mut index) = index();
    let probe = FakeProbe::new();
    let report =
        detect_and_store(index.conn_mut(), &probe, dir.path(), "/home/u", 1_000).expect("detect");
    assert_eq!(report.inserted, 0);
    assert_eq!(report.default_target_id, None);
    assert!(
        rows(index.conn()).is_empty(),
        "the ask tier is an empty table, not a stub row"
    );
}

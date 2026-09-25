#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! Launch-target resolution: five tiers in order, the first hit wins, and nothing means ask.

use codotheca_core::index::Index;
use codotheca_core::launch::catalogue::TargetKind;
use codotheca_core::launch::resolve::{resolve, TargetTier};

const SEED: &str = "
INSERT INTO project (id, name, seed_basename, created_at, updated_at) VALUES (1, 'p', 'p', 0, 0);
INSERT INTO location (id, project_id, kind, distro, path_bytes, path_key, path_display,
                      volume_key, store_key, presence, repo_kind)
     VALUES (1, 1, 'linux', '', X'2F61', X'2F61', '/a', 'v', 's', 'present', 'worktree');
";

fn target(
    conn: &rusqlite::Connection,
    name: &str,
    project: Option<i64>,
    location: Option<i64>,
    language: Option<&str>,
) {
    conn.execute(
        "INSERT INTO launch_target
             (project_id, location_id, language, kind, name, exec_bytes, args_json, cwd_mode,
              env_json, sort_index, detected, verify_state)
         VALUES (?1, ?2, ?3, 'editor', ?4, X'2F62696E2F65', '[\"{path}\"]', 'location', '{}', 0, 1, 'ok')",
        rusqlite::params![project, location, language, name],
    )
    .expect("insert");
}

fn open() -> (tempfile::TempDir, Index) {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    index.conn().execute_batch(SEED).expect("seed");
    (dir, index)
}

#[test]
fn the_five_tiers_resolve_in_order_and_first_hit_wins() {
    let (_d, index) = open();
    let conn = index.conn();
    target(conn, "global", None, None, None);
    assert_eq!(
        resolve(conn, 1, Some(1), Some("Rust"), TargetKind::Editor)
            .unwrap()
            .unwrap()
            .tier,
        TargetTier::Global
    );

    target(conn, "lang", None, None, Some("Rust"));
    assert_eq!(
        resolve(conn, 1, Some(1), Some("Rust"), TargetKind::Editor)
            .unwrap()
            .unwrap()
            .tier,
        TargetTier::Language
    );

    target(conn, "loc", None, Some(1), None);
    assert_eq!(
        resolve(conn, 1, Some(1), Some("Rust"), TargetKind::Editor)
            .unwrap()
            .unwrap()
            .tier,
        TargetTier::Location
    );

    target(conn, "proj", Some(1), None, None);
    let r = resolve(conn, 1, Some(1), Some("Rust"), TargetKind::Editor)
        .unwrap()
        .unwrap();
    assert_eq!(
        (r.tier, r.target.name.as_str()),
        (TargetTier::Project, "proj"),
        "a project outranks its own copies"
    );
}

#[test]
fn a_tier_with_no_row_is_skipped_not_guessed_at() {
    let (_d, index) = open();
    let conn = index.conn();
    target(conn, "proj", Some(1), None, None);
    // No location, language or global row exists at all.
    let r = resolve(conn, 1, Some(1), None, TargetKind::Editor)
        .unwrap()
        .unwrap();
    assert_eq!(r.tier, TargetTier::Project);
}

#[test]
fn an_unknown_language_falls_to_the_global_default_never_to_a_language_row() {
    let (_d, index) = open();
    let conn = index.conn();
    target(conn, "lang-rs", None, None, Some("Rust"));
    target(conn, "global", None, None, None);
    let r = resolve(conn, 1, Some(1), None, TargetKind::Editor)
        .unwrap()
        .unwrap();
    assert_eq!(
        (r.tier, r.target.name.as_str()),
        (TargetTier::Global, "global")
    );
}

#[test]
fn an_unknown_language_with_only_a_language_row_present_reaches_the_ask_tier() {
    let (_d, index) = open();
    let conn = index.conn();
    target(conn, "lang-rs", None, None, Some("Rust"));
    assert!(
        resolve(conn, 1, Some(1), None, TargetKind::Editor)
            .unwrap()
            .is_none(),
        "NULL primary_language must not take the first language row"
    );
}

#[test]
fn language_matching_is_exact_and_never_the_drawer_tag() {
    let (_d, index) = open();
    let conn = index.conn();
    target(conn, "lang-rs", None, None, Some("Rust"));
    target(conn, "global", None, None, None);
    assert_eq!(
        resolve(conn, 1, Some(1), Some("RS"), TargetKind::Editor)
            .unwrap()
            .unwrap()
            .tier,
        TargetTier::Global
    );
    assert_eq!(
        resolve(conn, 1, Some(1), Some("rust"), TargetKind::Editor)
            .unwrap()
            .unwrap()
            .tier,
        TargetTier::Global
    );
}

#[test]
fn within_a_tier_the_lowest_sort_index_of_that_kind_is_the_default() {
    let (_d, index) = open();
    let conn = index.conn();
    target(conn, "second", None, None, None);
    target(conn, "first", None, None, None);
    conn.execute(
        "UPDATE launch_target SET sort_index = 5 WHERE name = 'second'",
        [],
    )
    .expect("bump");
    conn.execute(
        "INSERT INTO launch_target (project_id, location_id, language, kind, name, exec_bytes,
             args_json, cwd_mode, env_json, sort_index, detected, verify_state)
         VALUES (NULL, NULL, NULL, 'terminal', 'term', X'2F62696E2F74', '[]', 'none', '{}', 0, 1, 'ok')",
        [],
    )
    .expect("terminal");
    let r = resolve(conn, 1, Some(1), None, TargetKind::Editor)
        .unwrap()
        .unwrap();
    assert_eq!(
        r.target.name, "first",
        "a terminal at sort_index 0 is not the editor default"
    );
}

#[test]
fn a_failing_verify_state_still_resolves_but_a_disabled_row_does_not() {
    let (_d, index) = open();
    let conn = index.conn();
    target(conn, "broken", Some(1), None, None);
    conn.execute(
        "UPDATE launch_target SET verify_state = 'missing' WHERE name = 'broken'",
        [],
    )
    .expect("break it");
    let r = resolve(conn, 1, Some(1), None, TargetKind::Editor)
        .unwrap()
        .unwrap();
    assert_eq!(
        r.target.name, "broken",
        "resolution must not consult verify_state"
    );

    conn.execute(
        "UPDATE launch_target SET disabled = 1 WHERE name = 'broken'",
        [],
    )
    .expect("disable");
    assert!(resolve(conn, 1, Some(1), None, TargetKind::Editor)
        .unwrap()
        .is_none());
}

#[test]
fn nothing_anywhere_is_the_ask_tier_and_is_a_none_not_an_error() {
    let (_d, index) = open();
    assert!(
        resolve(index.conn(), 1, Some(1), Some("Rust"), TargetKind::Editor)
            .unwrap()
            .is_none()
    );
}

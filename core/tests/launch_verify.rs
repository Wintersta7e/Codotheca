#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! Verifying launch targets: missing and not executable told apart, and no row ever disabled.

use codotheca_core::index::Index;
use codotheca_core::launch::verify::{verify_all, verify_path, VerifyState};

fn open() -> (tempfile::TempDir, Index) {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    (dir, index)
}

fn add(conn: &rusqlite::Connection, name: &str, exec: &[u8], language: Option<&str>) {
    conn.execute(
        "INSERT INTO launch_target (project_id, location_id, language, kind, name, exec_bytes,
             args_json, cwd_mode, env_json, sort_index, detected, verify_state)
         VALUES (NULL, NULL, ?3, 'editor', ?1, ?2, '[]', 'location', '{}', 0, 1, 'unverified')",
        rusqlite::params![name, exec, language],
    )
    .expect("insert");
}

/// A file the OS would actually run. Written rather than assumed: on Unix a plain
/// `fs::write` leaves mode 0644, which is `not_executable`, not `ok`.
fn executable_file(path: &std::path::Path) {
    std::fs::write(path, b"#!/bin/sh\n").expect("write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
}

#[test]
fn a_missing_executable_and_a_present_one_are_told_apart() {
    let dir = tempfile::tempdir().expect("tempdir");
    let present = dir.path().join("app");
    executable_file(&present);
    assert_eq!(verify_path(&present), VerifyState::Ok);
    assert_eq!(
        verify_path(&dir.path().join("absent")),
        VerifyState::Missing
    );
    assert_eq!(
        verify_path(dir.path()),
        VerifyState::NotExecutable,
        "a directory is not a target"
    );
}

#[cfg(unix)]
#[test]
fn a_present_file_with_no_execute_bit_is_not_executable_rather_than_missing() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().expect("tempdir");
    let plain = dir.path().join("data");
    std::fs::write(&plain, b"x").expect("write");
    std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    assert_eq!(verify_path(&plain), VerifyState::NotExecutable);
}

#[test]
fn every_row_is_verified_and_each_carries_its_own_state() {
    let (dir, mut index) = open();
    let present = dir.path().join("app");
    executable_file(&present);
    let good = codotheca_core::paths::path_bytes(&present);
    let bad = codotheca_core::paths::path_bytes(&dir.path().join("gone"));

    add(index.conn(), "global", &good, None);
    add(index.conn(), "lang", &bad, Some("Rust"));

    let out = verify_all(index.conn_mut(), 5_000).expect("verify");
    assert_eq!(out.len(), 2);
    let states: Vec<_> = out.iter().map(|v| v.verify_state).collect();
    assert!(
        states.contains(&VerifyState::Ok) && states.contains(&VerifyState::Missing),
        "the language row was covered by the global row's result"
    );
    for v in &out {
        assert_eq!(v.verified_at, 5_000);
    }
}

#[test]
fn the_stored_row_is_updated_so_the_menu_can_render_the_state() {
    let (dir, mut index) = open();
    add(
        index.conn(),
        "gone",
        &codotheca_core::paths::path_bytes(&dir.path().join("gone")),
        None,
    );
    verify_all(index.conn_mut(), 5_000).expect("verify");
    let (state, at): (String, i64) = index
        .conn()
        .query_row(
            "SELECT verify_state, verified_at FROM launch_target",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("row");
    assert_eq!((state.as_str(), at), ("missing", 5_000));
}

#[test]
fn a_disabled_row_is_still_verified() {
    let (dir, mut index) = open();
    add(
        index.conn(),
        "gone",
        &codotheca_core::paths::path_bytes(&dir.path().join("gone")),
        None,
    );
    index
        .conn()
        .execute("UPDATE launch_target SET disabled = 1", [])
        .expect("disable");
    let out = verify_all(index.conn_mut(), 5_000).expect("verify");
    assert_eq!(
        out.len(),
        1,
        "verification is unfiltered where resolution is filtered"
    );
}

#[test]
fn verification_never_disables_or_deletes_a_row() {
    let (dir, mut index) = open();
    add(
        index.conn(),
        "gone",
        &codotheca_core::paths::path_bytes(&dir.path().join("gone")),
        None,
    );
    verify_all(index.conn_mut(), 5_000).expect("verify");
    let (count, disabled): (i64, i64) = index
        .conn()
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(disabled), 0) FROM launch_target",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("row");
    assert_eq!(
        (count, disabled),
        (1, 0),
        "phase 1 has no destructive operation (§17)"
    );
}

//! J3 against a real index, and against a real repository.
//!
//! The fold from extension bytes to language bytes is unit-tested beside the code. What this
//! file adds is the two ends nothing else covers: that `TrackedInventory.paths` is really
//! populated by the git layer (the archetype has no other source), and that `commit_inventory`
//! writes the columns §1.2 declares.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::must_use_candidate,
    clippy::missing_panics_doc
)]

mod support;

use std::collections::BTreeMap;
use std::sync::Arc;

use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::SystemClock;
use codotheca_core::git::{GitBackend, GitSlots, JobClass, JobContext, SystemGit};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::jobs::j3_inventory::{commit_inventory, observe, InventoryFacts};
use codotheca_core::protocol::{LocationId, ProjectId};
use support::TestRepo;

fn fresh() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

fn insert_project(conn: &rusqlite::Connection, name: &str) -> ProjectId {
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at) VALUES (?1, ?1, 0, 0)",
        [name],
    )
    .unwrap();
    ProjectId(conn.last_insert_rowid())
}

fn insert_location(conn: &rusqlite::Connection, project: ProjectId) -> LocationId {
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                               store_key, presence, repo_kind)
         VALUES (?1, 'linux', x'2f77', x'2f77', '/w', 'store', 'present', 'worktree')",
        [project.0],
    )
    .unwrap();
    LocationId(conn.last_insert_rowid())
}

/// The archetype has no source but the tracked path set, so this is the test that would have
/// caught `TrackedInventory` silently dropping it.
#[test]
fn a_real_repository_yields_its_tracked_paths_and_language_bytes() {
    let repo = TestRepo::init();
    repo.write("Cargo.toml", b"[package]\nname = \"x\"\n");
    repo.write("src/lib.rs", b"pub fn a() {}\n");
    repo.write("README.md", b"# x\n");
    repo.git(&["add", "-A"]);
    repo.commit("first");

    let git = SystemGit::new(
        Arc::new(repo.exec()),
        Arc::new(GitSlots::new(4)),
        Arc::new(SystemClock::new()),
    );
    let cancel = CancelToken::new();
    let ctx = JobContext::new(JobClass::Background, &cancel, None);
    let handle = repo.handle();

    let inv = git.tracked_inventory(&handle, &ctx).unwrap();
    assert_eq!(inv.tracked_files, 3);
    let mut names: Vec<String> = inv
        .paths
        .iter()
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect();
    names.sort();
    assert_eq!(names, vec!["Cargo.toml", "README.md", "src/lib.rs"]);

    let facts = observe(&git, &handle, &ctx, repo.path()).unwrap();
    assert_eq!(facts.tracked_files, 3);
    assert!(facts.language_bytes.contains_key("Rust"));
    assert!(facts.language_bytes.contains_key("Markdown"));
    assert!(facts.worktree_newest_mtime.is_some());
}

#[test]
fn committing_an_inventory_writes_the_columns_section_one_declares() {
    let (_dir, mut conn) = fresh();
    let project = insert_project(&conn, "p");
    let location = insert_location(&conn, project);

    let mut language_bytes = BTreeMap::new();
    language_bytes.insert("Rust".to_owned(), 900_u64);
    language_bytes.insert("Markdown".to_owned(), 40_u64);

    let facts = InventoryFacts {
        tracked_files: 3,
        head_bytes: 940,
        language_bytes,
        sample_paths: vec!["Cargo.toml".to_owned(), "src/lib.rs".to_owned()],
        worktree_newest_mtime: Some(1_700_000_000),
    };

    let tx = conn.transaction().unwrap();
    commit_inventory(&tx, project, location, &facts).unwrap();
    tx.commit().unwrap();

    let (files, bytes, langs, primary, archetype): (i64, i64, String, Option<String>, String) =
        conn.query_row(
            "SELECT tracked_files, size_tracked_bytes, language_bytes, primary_language, archetype
               FROM project WHERE id = ?1",
            [project.0],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(files, 3);
    assert_eq!(bytes, 940);
    assert_eq!(primary.as_deref(), Some("Rust"), "markup is never primary");
    assert_eq!(archetype, "library");
    let parsed: BTreeMap<String, u64> = serde_json::from_str(&langs).unwrap();
    assert_eq!(parsed.get("Markdown").copied(), Some(40));

    let mtime: Option<i64> = conn
        .query_row(
            "SELECT worktree_newest_mtime FROM location WHERE id = ?1",
            [location.0],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(mtime, Some(1_700_000_000));
}

/// An inventory that recognised no programming language writes NULL, never a placeholder — a
/// language row must not resolve for a repository that has none (§4bis.2a).
#[test]
fn a_repository_with_no_programming_language_stores_null() {
    let (_dir, mut conn) = fresh();
    let project = insert_project(&conn, "p");
    let location = insert_location(&conn, project);

    let mut language_bytes = BTreeMap::new();
    language_bytes.insert("Markdown".to_owned(), 900_u64);

    let tx = conn.transaction().unwrap();
    commit_inventory(
        &tx,
        project,
        location,
        &InventoryFacts {
            tracked_files: 1,
            head_bytes: 900,
            language_bytes,
            sample_paths: vec!["README.md".to_owned()],
            worktree_newest_mtime: None,
        },
    )
    .unwrap();
    tx.commit().unwrap();

    let (primary, archetype): (Option<String>, String) = conn
        .query_row(
            "SELECT primary_language, archetype FROM project WHERE id = ?1",
            [project.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(primary, None);
    assert_eq!(archetype, "docs");
}

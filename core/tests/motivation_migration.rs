//! `0022_first_run_backfill.sql` — first run's end for an index that already holds a shelf
//! (§43.15). `MAX(project.created_at)` places every project the index held at or before the stamp,
//! so the upgrade marks none of them `NEW`; an empty index is left for the scan that stores its
//! first shelf (§43.10).
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;

use codotheca_core::art::testsupport::CollectingSink;
use codotheca_core::firstrun::first_run_completed_at;
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::projects::{dispatch_projects_command, ProjectsCtx};
use rusqlite::{params, Connection};

/// The migration's own text, compiled in: what it states is what the chain ran.
const MIGRATION: &str = include_str!("../migrations/0022_first_run_backfill.sql");

/// The version before this migration: the shipped chain up to and including `0021`.
const BEFORE: usize = 21;

const NOW: i64 = 1_781_179_200;

/// A database in `dir` at exactly `version` files applied, through the shipped set.
fn migrated_to(dir: &Path, version: usize) -> Connection {
    let mut conn = open_connection(&Index::db_path(dir)).unwrap();
    apply_all(&mut conn, &MIGRATIONS[..version]).unwrap();
    conn
}

/// Seeds one project per `created_at`, each with a present location so `projects.list` files
/// it under a live section rather than `notcloned`. Returns how many rows the index holds.
fn seed(conn: &Connection, created: &[i64]) -> i64 {
    for at in created {
        conn.execute(
            "INSERT INTO project (name, seed_basename, last_touched_at, created_at, updated_at)
             VALUES ('p' || ?1, 'p' || ?1, ?1, ?1, ?1)",
            params![at],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        let path = format!("/p/{at}");
        conn.execute(
            "INSERT INTO location (project_id, kind, distro, path_bytes, path_key,
                                   path_display, volume_key, store_key, presence, repo_kind)
             VALUES (?1, 'linux', '', CAST(?2 AS BLOB), CAST(?2 AS BLOB), ?2, 'v', 's',
                     'present', 'worktree')",
            params![id, path],
        )
        .unwrap();
    }
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM project", [], |r| r.get(0))
        .unwrap();
    eprintln!("motivation_migration: seeded {count} project(s)");
    assert!(
        count > 0,
        "a backfill over no seeded project proves nothing"
    );
    count
}

/// The ids `projects.list` answers for `is:new`, through the command's real entry.
fn new_ids(index: &Index) -> Vec<i64> {
    let sink = CollectingSink::default();
    let jobs = codotheca_core::jobs::NullJobSink;
    let mounts = codotheca_core::testing::FakeMountResolver::new();
    let ctx = ProjectsCtx {
        index,
        events: &sink,
        jobs: &jobs,
        mounts: &mounts,
        sync: &codotheca_core::sync::runner::NullSyncSink,
        now: NOW,
        tz_offset_min: 0,
    };
    let page = dispatch_projects_command(
        &ctx,
        "projects.list",
        serde_json::json!({ "query": "is:new" }),
    )
    .expect("owned")
    .expect("ok");
    page["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_i64().unwrap())
        .collect()
}

#[test]
fn ac_p4_43_15_the_backfill_writes_the_latest_created_at_over_a_shelf() {
    let dir = tempfile::tempdir().unwrap();
    let conn = migrated_to(dir.path(), BEFORE);
    seed(&conn, &[100, 300, 200]);
    assert_eq!(first_run_completed_at(&conn).unwrap(), None);
    drop(conn);

    // The real open path runs the rest of the chain.
    let index = Index::open_at(dir.path(), NOW).unwrap();
    let stamp = first_run_completed_at(index.conn()).unwrap();
    assert_eq!(stamp, Some(300));

    let later: i64 = index
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM project WHERE created_at > ?1",
            params![stamp.unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        later, 0,
        "every project the index held is at or before first run's end"
    );
    assert_eq!(
        new_ids(&index),
        Vec::<i64>::new(),
        "no project the index held reads NEW"
    );

    // And `is:new` is live over the stamp, or the empty answer above proves nothing: a discovery
    // after first run's end reads NEW.
    seed(index.conn(), &[400]);
    let discovered: i64 = index
        .conn()
        .query_row("SELECT id FROM project WHERE created_at = 400", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(new_ids(&index), vec![discovered]);
}

#[test]
fn ac_p4_43_15_an_empty_index_is_left_for_path_two() {
    let dir = tempfile::tempdir().unwrap();
    let conn = migrated_to(dir.path(), MIGRATIONS.len());
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM project", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(first_run_completed_at(&conn).unwrap(), None);
}

#[test]
fn an_index_already_stamped_keeps_its_stamp() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = migrated_to(dir.path(), BEFORE);
    seed(&conn, &[100, 300, 200]);
    conn.execute(
        "INSERT INTO app_meta (k, v) VALUES ('first_run_completed_at', '50')",
        [],
    )
    .unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    assert_eq!(first_run_completed_at(&conn).unwrap(), Some(50));
}

#[test]
fn the_backfill_reads_no_clock_and_runs_no_pragma() {
    // Comments may name what the statement avoids; only the statement is checked.
    let statement: String = MIGRATION
        .lines()
        .map(|line| line.split("--").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    assert!(
        statement.contains("insert into app_meta"),
        "the statement was not read: {statement:?}"
    );
    for banned in [
        "pragma",
        "strftime",
        "unixepoch",
        "julianday",
        "'now'",
        "create ",
        "drop ",
    ] {
        assert!(!statement.contains(banned), "the backfill holds {banned:?}");
    }
}

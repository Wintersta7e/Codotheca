//! §28.2's producers — the `todo_marker` builder over §29's occurrences.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::debt::markers::{build_items, project_ordinals};
use codotheca_core::debt::store::{SqliteDebtStore, SweepEffect};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::jobs::j7_markers::{ContentGates, ContentOccurrence};
use codotheca_core::jobs::markers::Marker;
use codotheca_core::protocol::{LocationId, ProjectId};

const HEAD: &str = "head0000";

fn fresh() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

fn insert_project(conn: &rusqlite::Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
         VALUES (?1, ?1, 'abc123', 1, 1)",
        [name],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn insert_location(conn: &rusqlite::Connection, project: i64) -> LocationId {
    let n: i64 = conn
        .query_row("SELECT count(*) FROM location", [], |r| r.get(0))
        .unwrap();
    let path = format!("/copy-{n}");
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind)
         VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree')",
        rusqlite::params![project, path.as_bytes(), path],
    )
    .unwrap();
    LocationId(conn.last_insert_rowid())
}

/// §29's per-project scan row. `complete_head_oid == head_oid` is what makes the sweep
/// `complete`; anything else is `partial`, and an **absent row** is *never observed*.
fn content_scan(conn: &rusqlite::Connection, project: i64, complete: bool) {
    conn.execute(
        "INSERT INTO project_content_scan
            (project_id, head_oid, complete_head_oid, blobs_total, blobs_pending,
             predicate_version, has_readme, has_license, has_tests, has_ci,
             presence_observed_at, enumerated_at, completed_at)
         VALUES (?1, ?2, ?3, 4, 0, 1, 'present', 'present', 'present', 'present', 1, 1, 1)
         ON CONFLICT(project_id) DO UPDATE SET complete_head_oid = excluded.complete_head_oid",
        rusqlite::params![project, HEAD, if complete { Some(HEAD) } else { None }],
    )
    .unwrap();
}

fn occurrence(path: &str, line: u32, salient: &str) -> ContentOccurrence {
    occurrence_in(&format!("blob-of-{salient}"), path, line, salient)
}

/// The same helper with the blob named, because *one blob reachable at two paths* is the case
/// the per-project ordinal exists for and it cannot be expressed when the blob is derived from
/// the marker text.
fn occurrence_in(blob: &str, path: &str, line: u32, salient: &str) -> ContentOccurrence {
    ContentOccurrence {
        blob_oid: blob.to_owned(),
        path_bytes: path.as_bytes().to_vec(),
        line,
        column: 1,
        marker: Marker::Todo,
        salient_sha256: format!("{salient:0>64}"),
        salient_text_capped: format!("TODO: {salient}"),
    }
}

fn gates(runs: bool) -> ContentGates {
    ContentGates {
        is_reference: Some(!runs),
        granted: true,
        compute_suppressed: false,
    }
}

fn items(conn: &rusqlite::Connection, project: i64) -> Vec<(String, String, i64)> {
    let mut st = conn
        .prepare(
            "SELECT fingerprint, path_display, line FROM debt_item
              WHERE project_id = ?1 ORDER BY fingerprint",
        )
        .unwrap();
    st.query_map([project], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

// ---------------------------------------------------------------------------------------------
// The per-project ordinal
// ---------------------------------------------------------------------------------------------

/// **`AC-P3-28-4`.** Two identical marker texts in one file are **two items**, distinguished by
/// the ordinal, and deleting one closes **exactly one**.
#[test]
fn ac_p3_28_4_two_identical_markers_are_two_items() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let loc = insert_location(&conn, p);
    content_scan(&conn, p, true);
    let store = SqliteDebtStore;

    let both = vec![occurrence("a.rs", 4, "same"), occurrence("a.rs", 9, "same")];
    assert_eq!(project_ordinals(&both), vec![0, 1]);

    let tx = conn.transaction().unwrap();
    let opened = build_items(&tx, ProjectId(p), Some(loc), gates(true), &both, 10, &store).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        opened.opened.len(),
        2,
        "two identical texts collapsed to one item"
    );
    assert_eq!(items(&conn, p).len(), 2);

    // Delete the second one. The survivor renumbers to ordinal 0, so the closure attributes to
    // the twin — *ordinal churn*, accepted: the count is right and the payout is right.
    let tx = conn.transaction().unwrap();
    let one = vec![occurrence("a.rs", 4, "same")];
    let after = build_items(&tx, ProjectId(p), Some(loc), gates(true), &one, 20, &store).unwrap();
    tx.commit().unwrap();

    assert_eq!(
        after.closed.len(),
        1,
        "deleting one marker closed {} items",
        after.closed.len()
    );
    assert_eq!(items(&conn, p).len(), 1);
}

/// **The case that proves the ordinal is per project and not per blob.** One blob reachable at
/// two paths contributes its occurrences **twice**, so two occurrences at two paths are four
/// items. `ordinal_in_blob` would collapse each pair onto one identity.
#[test]
fn one_blob_at_two_paths_contributes_twice() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let loc = insert_location(&conn, p);
    content_scan(&conn, p, true);
    let store = SqliteDebtStore;

    // **One** blob, holding two markers, reachable at `a.rs` and `b.rs` — so
    // `occurrences_for_project` emits its two findings once per path, four in all.
    let all = vec![
        occurrence_in("blob-1", "a.rs", 1, "first"),
        occurrence_in("blob-1", "a.rs", 2, "second"),
        occurrence_in("blob-1", "b.rs", 1, "first"),
        occurrence_in("blob-1", "b.rs", 2, "second"),
    ];
    assert_eq!(
        project_ordinals(&all),
        vec![0, 0, 1, 1],
        "the ordinal must count within a salient hash across the whole project"
    );

    let tx = conn.transaction().unwrap();
    let effect = build_items(&tx, ProjectId(p), Some(loc), gates(true), &all, 10, &store).unwrap();
    tx.commit().unwrap();

    assert_eq!(
        effect.opened.len(),
        4,
        "a blob at two paths collapsed onto one project position"
    );
    assert_eq!(items(&conn, p).len(), 4);
}

/// **`AC-P3-28-3`.** A line move and a file rename close nothing and update the attributes.
#[test]
fn ac_p3_28_3_a_line_move_and_a_rename_close_nothing() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let loc = insert_location(&conn, p);
    content_scan(&conn, p, true);
    let store = SqliteDebtStore;

    let tx = conn.transaction().unwrap();
    build_items(
        &tx,
        ProjectId(p),
        Some(loc),
        gates(true),
        &[occurrence("old/a.rs", 4, "text")],
        10,
        &store,
    )
    .unwrap();
    tx.commit().unwrap();
    let before = items(&conn, p);

    let tx = conn.transaction().unwrap();
    let effect = build_items(
        &tx,
        ProjectId(p),
        Some(loc),
        gates(true),
        &[occurrence("new/b.rs", 91, "text")],
        20,
        &store,
    )
    .unwrap();
    tx.commit().unwrap();

    assert!(effect.closed.is_empty(), "a rename closed an item");
    assert!(effect.opened.is_empty(), "a rename opened a second item");
    assert_eq!(effect.refreshed, 1);

    let after = items(&conn, p);
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].0, before[0].0, "the fingerprint moved");
    assert_eq!(after[0].1, "new/b.rs");
    assert_eq!(after[0].2, 91);
}

/// **`AC-P3-28-2`.** A budget cut-off is `partial`, and a partial sweep over a corpus whose
/// marker set shrank between runs **opens items and closes none**: an item it did not reach looks
/// exactly like an item that is gone.
#[test]
fn ac_p3_28_2_a_budget_cut_off_opens_and_closes_nothing() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let loc = insert_location(&conn, p);
    content_scan(&conn, p, true);
    let store = SqliteDebtStore;

    let tx = conn.transaction().unwrap();
    build_items(
        &tx,
        ProjectId(p),
        Some(loc),
        gates(true),
        &[occurrence("a.rs", 1, "one"), occurrence("b.rs", 1, "two")],
        10,
        &store,
    )
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(items(&conn, p).len(), 2);

    // The head moves and the scan is cut off mid-way: `complete_head_oid` no longer matches.
    conn.execute(
        "UPDATE project_content_scan SET complete_head_oid = NULL WHERE project_id = ?1",
        [p],
    )
    .unwrap();

    let tx = conn.transaction().unwrap();
    let effect = build_items(
        &tx,
        ProjectId(p),
        Some(loc),
        gates(true),
        &[occurrence("a.rs", 1, "one"), occurrence("c.rs", 1, "three")],
        20,
        &store,
    )
    .unwrap();
    tx.commit().unwrap();

    assert_eq!(effect.opened.len(), 1, "the new marker did not open");
    assert!(
        effect.closed.is_empty(),
        "a partial sweep closed {} items",
        effect.closed.len()
    );
    assert_eq!(items(&conn, p).len(), 3);

    let outcome: String = conn
        .query_row(
            "SELECT outcome FROM debt_sweep WHERE project_id = ?1 AND source = 'todo_marker'",
            [p],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(outcome, "partial");
}

/// **`AC-P3-28-11`'s *not computed* half.** A project whose `content_sweep_state` is `None` gets
/// **no `debt_sweep` row at all** — `None` is *never observed* and is not an outcome, which is
/// what makes an empty item list render as *not computed* rather than as zero.
#[test]
fn ac_p3_28_11_never_observed_writes_no_sweep_row() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let loc = insert_location(&conn, p);
    let store = SqliteDebtStore;
    // Deliberately no `project_content_scan` row.

    let tx = conn.transaction().unwrap();
    let effect = build_items(
        &tx,
        ProjectId(p),
        Some(loc),
        gates(true),
        &[occurrence("a.rs", 1, "one")],
        10,
        &store,
    )
    .unwrap();
    tx.commit().unwrap();

    assert_eq!(
        effect,
        SweepEffect::default(),
        "an unobserved project was swept"
    );
    let sweeps: i64 = conn
        .query_row("SELECT count(*) FROM debt_sweep", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sweeps, 0, "a never-observed project wrote a sweep row");
    assert!(items(&conn, p).is_empty());
}

/// A `Reference` project's sweep is `skipped_reference` and carries no count: a gate that
/// declined to look is not a look that found nothing.
#[test]
fn a_reference_project_writes_skipped_reference_with_no_count() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let loc = insert_location(&conn, p);
    content_scan(&conn, p, true);
    let store = SqliteDebtStore;

    let tx = conn.transaction().unwrap();
    build_items(
        &tx,
        ProjectId(p),
        Some(loc),
        gates(false),
        &[occurrence("a.rs", 1, "one")],
        10,
        &store,
    )
    .unwrap();
    tx.commit().unwrap();

    let (outcome, count): (String, Option<i64>) = conn
        .query_row(
            "SELECT outcome, item_count FROM debt_sweep WHERE project_id = ?1",
            [p],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(outcome, "skipped_reference");
    assert_eq!(count, None, "a skipped sweep carried a count");
}

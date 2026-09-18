//! §28.2's producers — the `todo_marker` builder over §29's occurrences.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::debt::markers::{build_items, project_ordinals};
use codotheca_core::debt::singletons::evaluate_singletons;
use codotheca_core::debt::store::{SqliteDebtStore, SweepEffect};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::jobs::j7_markers::{ContentGates, ContentOccurrence};
use codotheca_core::jobs::markers::Marker;
use codotheca_core::protocol::{LocationId, ProjectId};
use rusqlite::OptionalExtension as _;

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

// ---------------------------------------------------------------------------------------------
// §28.2's singleton evaluator — R124's six sources with no producer until now
// ---------------------------------------------------------------------------------------------

fn presence(conn: &rusqlite::Connection, project: i64, readme: &str, license: &str, tests: &str) {
    conn.execute(
        "INSERT INTO project_content_scan
            (project_id, head_oid, complete_head_oid, blobs_total, blobs_pending,
             predicate_version, has_readme, has_license, has_tests, has_ci,
             presence_observed_at, enumerated_at, completed_at)
         VALUES (?1, ?2, ?2, 4, 0, 1, ?3, ?4, ?5, 'present', 1, 1, 1)
         ON CONFLICT(project_id) DO UPDATE SET
            has_readme = excluded.has_readme,
            has_license = excluded.has_license,
            has_tests = excluded.has_tests",
        rusqlite::params![project, HEAD, readme, license, tests],
    )
    .unwrap();
}

fn sweep_of(
    conn: &rusqlite::Connection,
    project: i64,
    source: &str,
) -> Option<(String, Option<i64>)> {
    conn.query_row(
        "SELECT outcome, item_count FROM debt_sweep WHERE project_id = ?1 AND source = ?2",
        rusqlite::params![project, source],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .unwrap()
}

fn items_of(conn: &rusqlite::Connection, project: i64, source: &str) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM debt_item WHERE project_id = ?1 AND source = ?2",
        rusqlite::params![project, source],
        |r| r.get(0),
    )
    .unwrap()
}

/// **§28.2a, and the live instance it was written from.** J6 assigns
/// `readme_excerpt = read_capped(…)`, which returns `None` on **any** open or read failure, and
/// the `peek_cache` row is then written with a NULL excerpt whose convention is *"no README in
/// this repository"*. **A debt producer reading that would open `missing_readme` on a repository
/// that has one.** The evaluator reads J7's HEAD-basis path predicate, where presence is decided
/// by the path existing and no file is opened at all.
#[test]
fn an_unreadable_readme_opens_no_item() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    insert_location(&conn, p);
    // Present at HEAD…
    presence(&conn, p, "present", "present", "present");
    // …and unreadable on disk, which is exactly what J6 records as a NULL excerpt.
    conn.execute(
        "INSERT INTO peek_cache (project_id, readme_excerpt, computed_at) VALUES (?1, NULL, 1)",
        [p],
    )
    .unwrap();

    let tx = conn.transaction().unwrap();
    let effect = evaluate_singletons(&tx, ProjectId(p), 10, &SqliteDebtStore).unwrap();
    tx.commit().unwrap();

    assert!(
        effect.opened.is_empty(),
        "opened {:?} against a repository that has a README",
        effect.opened
    );
    assert_eq!(items_of(&conn, p, "missing_readme"), 0);
    assert_eq!(
        sweep_of(&conn, p, "missing_readme"),
        Some(("complete".into(), Some(0)))
    );
}

/// A `not_read` answer is unknown and **never a false**: it writes `unobservable` and opens
/// nothing. An input that was never observed opens nothing **and marks nothing**.
#[test]
fn a_not_read_presence_writes_unobservable_and_opens_nothing() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    insert_location(&conn, p);
    presence(&conn, p, "not_read", "not_read", "not_read");

    let tx = conn.transaction().unwrap();
    let effect = evaluate_singletons(&tx, ProjectId(p), 10, &SqliteDebtStore).unwrap();
    tx.commit().unwrap();

    assert!(effect.opened.is_empty());
    for source in ["missing_readme", "missing_license", "missing_tests"] {
        assert_eq!(
            items_of(&conn, p, source),
            0,
            "{source} opened on an unknown"
        );
        assert_eq!(
            sweep_of(&conn, p, source),
            Some(("unobservable".into(), None)),
            "{source}"
        );
    }
}

/// `presence_for_project` returning `None` is **row-absent**, which is neither `absent` nor
/// `not_read`. The two reach the same outcome and are **different rows**, which is correct and is
/// asserted so nobody collapses them.
#[test]
fn a_row_absent_presence_is_unobservable_and_is_not_not_read() {
    let (_d, mut conn) = fresh();
    let absent_row = insert_project(&conn, "no-row");
    insert_location(&conn, absent_row);
    let not_read = insert_project(&conn, "not-read");
    insert_location(&conn, not_read);
    presence(&conn, not_read, "not_read", "not_read", "not_read");

    let tx = conn.transaction().unwrap();
    for p in [absent_row, not_read] {
        evaluate_singletons(&tx, ProjectId(p), 10, &SqliteDebtStore).unwrap();
    }
    tx.commit().unwrap();

    for p in [absent_row, not_read] {
        assert_eq!(
            sweep_of(&conn, p, "missing_readme"),
            Some(("unobservable".into(), None))
        );
        assert_eq!(items_of(&conn, p, "missing_readme"), 0);
    }
    // Two rows, not one: the projects are distinct and each carries its own sweep.
    let rows: i64 = conn
        .query_row(
            "SELECT count(*) FROM debt_sweep WHERE source = 'missing_readme'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 2);
}

/// An absent README **is** a positive observation that the predicate is false, so it opens.
#[test]
fn an_absent_readme_opens_one_item() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    insert_location(&conn, p);
    presence(&conn, p, "absent", "present", "absent");

    let tx = conn.transaction().unwrap();
    let effect = evaluate_singletons(&tx, ProjectId(p), 10, &SqliteDebtStore).unwrap();
    tx.commit().unwrap();

    assert_eq!(effect.opened.len(), 2, "opened {:?}", effect.opened);
    assert_eq!(items_of(&conn, p, "missing_readme"), 1);
    assert_eq!(items_of(&conn, p, "missing_tests"), 1);
    assert_eq!(items_of(&conn, p, "missing_license"), 0);
    assert_eq!(
        sweep_of(&conn, p, "missing_readme"),
        Some(("complete".into(), Some(1)))
    );
}

#[test]
fn ahead_opens_unpushed_commits_and_null_opens_nothing() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let loc = insert_location(&conn, p);

    // NULL: never observed. `0` would be a claim nobody measured.
    let tx = conn.transaction().unwrap();
    evaluate_singletons(&tx, ProjectId(p), 10, &SqliteDebtStore).unwrap();
    tx.commit().unwrap();
    assert_eq!(items_of(&conn, p, "unpushed_commits"), 0);
    assert_eq!(
        sweep_of(&conn, p, "unpushed_commits"),
        Some(("unobservable".into(), None))
    );

    conn.execute("UPDATE location SET ahead = 3 WHERE id = ?1", [loc.0])
        .unwrap();
    let tx = conn.transaction().unwrap();
    evaluate_singletons(&tx, ProjectId(p), 20, &SqliteDebtStore).unwrap();
    tx.commit().unwrap();
    assert_eq!(items_of(&conn, p, "unpushed_commits"), 1);

    // Pushed: the predicate holds, the item closes, and the sweep says it looked.
    conn.execute("UPDATE location SET ahead = 0 WHERE id = ?1", [loc.0])
        .unwrap();
    let tx = conn.transaction().unwrap();
    let effect = evaluate_singletons(&tx, ProjectId(p), 30, &SqliteDebtStore).unwrap();
    tx.commit().unwrap();
    assert_eq!(effect.closed.len(), 1);
    assert_eq!(items_of(&conn, p, "unpushed_commits"), 0);
    assert_eq!(
        sweep_of(&conn, p, "unpushed_commits"),
        Some(("complete".into(), Some(0)))
    );
}

/// **R145's recorded fallback**, because there is no `default_branch` column to read: the latest
/// **concluded** run (`started_at` descending, `conclusion IS NOT NULL`) on the primary location's
/// `location.branch`, and `Unobservable` otherwise. A run still in flight opens nothing.
#[test]
fn the_latest_concluded_run_on_the_primary_branch_decides_ci_red() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let loc = insert_location(&conn, p);
    conn.execute("UPDATE location SET branch = 'main' WHERE id = ?1", [loc.0])
        .unwrap();
    conn.execute(
        "UPDATE project SET provider = 'github', provider_repo_id = 'r1' WHERE id = ?1",
        [p],
    )
    .unwrap();

    // No run at all: never observed.
    let tx = conn.transaction().unwrap();
    evaluate_singletons(&tx, ProjectId(p), 10, &SqliteDebtStore).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        sweep_of(&conn, p, "ci_red"),
        Some(("unobservable".into(), None))
    );

    // A run still in flight is not a reading.
    conn.execute(
        "INSERT INTO remote_ci_run
            (provider, provider_repo_id, run_id, workflow_name, conclusion, branch, run_number,
             started_at)
         VALUES ('github', 'r1', 9, 'build', NULL, 'main', 9, 900)",
        [],
    )
    .unwrap();
    let tx = conn.transaction().unwrap();
    evaluate_singletons(&tx, ProjectId(p), 20, &SqliteDebtStore).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        items_of(&conn, p, "ci_red"),
        0,
        "an unfinished run opened an item"
    );
    assert_eq!(
        sweep_of(&conn, p, "ci_red"),
        Some(("unobservable".into(), None))
    );

    // An older success and a newer failure: the newer one decides.
    conn.execute(
        "INSERT INTO remote_ci_run
            (provider, provider_repo_id, run_id, workflow_name, conclusion, branch, run_number,
             started_at)
         VALUES ('github', 'r1', 1, 'build', 'success', 'main', 1, 100),
                ('github', 'r1', 2, 'build', 'failure', 'main', 2, 200)",
        [],
    )
    .unwrap();
    let tx = conn.transaction().unwrap();
    evaluate_singletons(&tx, ProjectId(p), 30, &SqliteDebtStore).unwrap();
    tx.commit().unwrap();
    assert_eq!(items_of(&conn, p, "ci_red"), 1);

    // A run on another branch is not this project's answer.
    conn.execute(
        "INSERT INTO remote_ci_run
            (provider, provider_repo_id, run_id, workflow_name, conclusion, branch, run_number,
             started_at)
         VALUES ('github', 'r1', 3, 'build', 'success', 'topic', 3, 300)",
        [],
    )
    .unwrap();
    let tx = conn.transaction().unwrap();
    evaluate_singletons(&tx, ProjectId(p), 40, &SqliteDebtStore).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        items_of(&conn, p, "ci_red"),
        1,
        "another branch closed the item"
    );
}

/// **Deviation 1, PROVISIONAL.** `location.tag_count` is added by `0015_completion.sql`, which is
/// p3-31's in wave 4, and no plan may take another's migration number. The arm is declared and
/// inert: it writes `unobservable` and opens nothing in **every** case.
///
/// **`unobservable` here is the absence of the column, not a reading of it**, and this assertion
/// is expected to be replaced by p3-31 in the same change that lands `tag_count`.
#[test]
fn no_release_is_declared_and_inert_until_the_column_exists() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    insert_location(&conn, p);
    presence(&conn, p, "present", "present", "present");

    let tx = conn.transaction().unwrap();
    evaluate_singletons(&tx, ProjectId(p), 10, &SqliteDebtStore).unwrap();
    tx.commit().unwrap();

    assert_eq!(items_of(&conn, p, "no_release"), 0);
    assert_eq!(
        sweep_of(&conn, p, "no_release"),
        Some(("unobservable".into(), None))
    );

    // The column really is absent, so the arm could not read it even if it tried.
    let has_column: i64 = conn
        .query_row(
            "SELECT count(*) FROM pragma_table_info('location') WHERE name = 'tag_count'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        has_column, 0,
        "tag_count exists — p3-31 landed; fill the arm"
    );
}

/// A second settle with no input change opens nothing, closes nothing and adds no row.
#[test]
fn a_second_settle_with_no_change_writes_no_row() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    insert_location(&conn, p);
    presence(&conn, p, "absent", "present", "present");

    let tx = conn.transaction().unwrap();
    let first = evaluate_singletons(&tx, ProjectId(p), 10, &SqliteDebtStore).unwrap();
    tx.commit().unwrap();
    assert_eq!(first.opened.len(), 1);
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM debt_item", [], |r| r.get(0))
        .unwrap();

    let tx = conn.transaction().unwrap();
    let second = evaluate_singletons(&tx, ProjectId(p), 20, &SqliteDebtStore).unwrap();
    tx.commit().unwrap();

    assert!(
        second.opened.is_empty(),
        "a second settle opened a duplicate"
    );
    assert!(second.closed.is_empty());
    let after: i64 = conn
        .query_row("SELECT count(*) FROM debt_item", [], |r| r.get(0))
        .unwrap();
    assert_eq!(after, rows);
}

/// **Step 7's gate.** The evaluator reads `peek_cache` **nowhere**: J6's NULL excerpt means *any*
/// open or read failure and its own convention reads it as *no README*, so a producer reading it
/// would open `missing_readme` on a repository that has one.
///
/// It prints the file count scanned and **fails at zero** — a gate whose passing run scans nothing
/// is a failing gate.
#[test]
fn the_debt_module_reads_peek_cache_nowhere() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/debt");
    let mut scanned = 0_usize;
    let mut offenders = Vec::new();
    for entry in std::fs::read_dir(&root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        scanned += 1;
        // The doc comments that *explain* the ban name the table, so only a SQL-shaped use
        // counts: the string literals are where a read would live.
        for line in text.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if line.contains("peek_cache") {
                offenders.push(format!("{}: {}", path.display(), line.trim()));
            }
        }
    }
    eprintln!("peek_cache gate: scanned {scanned} file(s) under core/src/debt/");
    assert!(
        scanned > 0,
        "scanned zero files, so this gate proved nothing"
    );
    assert!(
        offenders.is_empty(),
        "the debt module reads peek_cache: {offenders:?}"
    );
}

//! §28.2's producers — the `todo_marker` builder over §29's occurrences.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::debt::abandoned::abandoned_conjunct;
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
        rusqlite::params![project, HEAD, complete.then_some(HEAD)],
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

const fn gates(runs: bool) -> ContentGates {
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
    let resweep_tx = conn.transaction().unwrap();
    let one = vec![occurrence("a.rs", 4, "same")];
    let after = build_items(
        &resweep_tx,
        ProjectId(p),
        Some(loc),
        gates(true),
        &one,
        20,
        &store,
    )
    .unwrap();
    resweep_tx.commit().unwrap();

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

    let resweep_tx = conn.transaction().unwrap();
    let effect = build_items(
        &resweep_tx,
        ProjectId(p),
        Some(loc),
        gates(true),
        &[occurrence("new/b.rs", 91, "text")],
        20,
        &store,
    )
    .unwrap();
    resweep_tx.commit().unwrap();

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

    let partial_tx = conn.transaction().unwrap();
    let effect = build_items(
        &partial_tx,
        ProjectId(p),
        Some(loc),
        gates(true),
        &[occurrence("a.rs", 1, "one"), occurrence("c.rs", 1, "three")],
        20,
        &store,
    )
    .unwrap();
    partial_tx.commit().unwrap();

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
    let ahead_tx = conn.transaction().unwrap();
    evaluate_singletons(&ahead_tx, ProjectId(p), 20, &SqliteDebtStore).unwrap();
    ahead_tx.commit().unwrap();
    assert_eq!(items_of(&conn, p, "unpushed_commits"), 1);

    // Pushed: the predicate holds, the item closes, and the sweep says it looked.
    conn.execute("UPDATE location SET ahead = 0 WHERE id = ?1", [loc.0])
        .unwrap();
    let pushed_tx = conn.transaction().unwrap();
    let effect = evaluate_singletons(&pushed_tx, ProjectId(p), 30, &SqliteDebtStore).unwrap();
    pushed_tx.commit().unwrap();
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
    let in_flight_tx = conn.transaction().unwrap();
    evaluate_singletons(&in_flight_tx, ProjectId(p), 20, &SqliteDebtStore).unwrap();
    in_flight_tx.commit().unwrap();
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
    let failure_tx = conn.transaction().unwrap();
    evaluate_singletons(&failure_tx, ProjectId(p), 30, &SqliteDebtStore).unwrap();
    failure_tx.commit().unwrap();
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
    let other_branch_tx = conn.transaction().unwrap();
    evaluate_singletons(&other_branch_tx, ProjectId(p), 40, &SqliteDebtStore).unwrap();
    other_branch_tx.commit().unwrap();
    assert_eq!(
        items_of(&conn, p, "ci_red"),
        1,
        "another branch closed the item"
    );
}

/// **Deviation 1, discharged.** The arm was declared inert because `location.tag_count` did not
/// exist; `0015_completion.sql` lands it and the body now reads it.
///
/// `unobservable` here is a **reading** and no longer the absence of a column: a copy whose
/// refstate J1 has never persisted carries NULL, and NULL is *never observed*. The predicate's
/// three branches are `AC-P3-31-11`'s, in `core/tests/acceptance_completion.rs`; what this
/// asserts is the part that belongs to §28 — that the arm's answer reaches the sweep row.
#[test]
fn no_release_reads_the_column_and_an_unpersisted_copy_is_unobservable() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let loc = insert_location(&conn, p);
    presence(&conn, p, "present", "present", "present");

    // The column exists and holds NULL, which is what J1 not having run looks like.
    let has_column: i64 = conn
        .query_row(
            "SELECT count(*) FROM pragma_table_info('location') WHERE name = 'tag_count'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(has_column, 1, "0015_completion.sql declares tag_count");

    let tx = conn.transaction().unwrap();
    evaluate_singletons(&tx, ProjectId(p), 10, &SqliteDebtStore).unwrap();
    tx.commit().unwrap();
    assert_eq!(items_of(&conn, p, "no_release"), 0);
    assert_eq!(
        sweep_of(&conn, p, "no_release"),
        Some(("unobservable".into(), None))
    );

    // A persisted zero on a copy that is not shallow is the opposite answer, and it reaches the
    // sweep row rather than stopping at the arm.
    conn.execute("UPDATE location SET tag_count = 0 WHERE id = ?1", [loc.0])
        .unwrap();
    let zero_tx = conn.transaction().unwrap();
    evaluate_singletons(&zero_tx, ProjectId(p), 20, &SqliteDebtStore).unwrap();
    zero_tx.commit().unwrap();
    assert_eq!(items_of(&conn, p, "no_release"), 1);
    assert_eq!(
        sweep_of(&conn, p, "no_release"),
        Some(("complete".into(), Some(1)))
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

    let second_tx = conn.transaction().unwrap();
    let second = evaluate_singletons(&second_tx, ProjectId(p), 20, &SqliteDebtStore).unwrap();
    second_tx.commit().unwrap();

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

// ---------------------------------------------------------------------------------------------
// `abandoned_with_debt` — the conjunct that must not satisfy itself
// ---------------------------------------------------------------------------------------------

fn set_abandoned(conn: &rusqlite::Connection, project: i64, abandoned: bool) {
    conn.execute(
        "UPDATE project SET condition_signal = ?2 WHERE id = ?1",
        rusqlite::params![project, if abandoned { "abandoned" } else { "idle" }],
    )
    .unwrap();
}

/// §29.8's content-scan grant. `todo_marker` is `off` without it (R128/F8), and an off check's
/// items are set aside — so a test counting a planted `todo_marker` item grants it first.
fn grant_content_scan(conn: &rusqlite::Connection) {
    conn.execute(
        "INSERT INTO app_meta (k, v) VALUES ('content_scan_enabled', '1')
         ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        [],
    )
    .unwrap();
}

/// §30.9 — one check switched off, which sets its items aside.
fn switch_off(conn: &rusqlite::Connection, check: codotheca_core::protocol::DebtSource) {
    let tx = conn.unchecked_transaction().unwrap();
    codotheca_core::health::switches::write_switches(
        &tx,
        &[codotheca_core::protocol::HealthCheckSwitch {
            check,
            enabled: false,
        }],
    )
    .unwrap();
    tx.commit().unwrap();
}

/// **The conjunct counts only what the reading counts.** A project whose only open scored items
/// belong to checks the reading sets aside — ungranted, switched off, not applicable — has no
/// outstanding work the reading speaks for, so `abandoned_with_debt` does not open.
#[test]
fn a_set_aside_item_does_not_light_the_conjunct() {
    use codotheca_core::protocol::DebtSource;

    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    insert_location(&conn, p);
    set_abandoned(&conn, p, true);
    // Three causes: `todo_marker` with no grant, `missing_license` switched off, and
    // `missing_tests` on a docs project, whose archetype proposes it N/A.
    plant_item(&conn, p, "todo_marker", "scored");
    plant_item(&conn, p, "missing_license", "scored");
    switch_off(&conn, DebtSource::MissingLicense);
    conn.execute("UPDATE project SET archetype = 'docs' WHERE id = ?1", [p])
        .unwrap();
    plant_item(&conn, p, "missing_tests", "scored");

    let tx = conn.transaction().unwrap();
    let lit = abandoned_conjunct(&tx, ProjectId(p)).unwrap();
    evaluate_singletons(&tx, ProjectId(p), 10, &SqliteDebtStore).unwrap();
    tx.commit().unwrap();
    eprintln!(
        "three set-aside items: conjunct {lit}, abandoned_with_debt items {}",
        items_of(&conn, p, "abandoned_with_debt")
    );
    assert!(!lit, "the conjunct counted an item the reading sets aside");
    assert_eq!(items_of(&conn, p, "abandoned_with_debt"), 0);

    // The control: granted, `todo_marker` is back inside the reading, and the conjunct lights.
    grant_content_scan(&conn);
    let granted_tx = conn.transaction().unwrap();
    assert!(abandoned_conjunct(&granted_tx, ProjectId(p)).unwrap());
    evaluate_singletons(&granted_tx, ProjectId(p), 20, &SqliteDebtStore).unwrap();
    granted_tx.commit().unwrap();
    assert_eq!(items_of(&conn, p, "abandoned_with_debt"), 1);
}

/// Open one item of `source` with the given scoring, outside the evaluator, so the conjunct is
/// tested against a stored set rather than against whatever the arms happened to write.
fn plant_item(conn: &rusqlite::Connection, project: i64, source: &str, scoring: &str) {
    conn.execute(
        "INSERT INTO debt_item (project_id, subject_key, source, fingerprint, state, scoring,
                                first_seen_at, last_seen_at)
         VALUES (?1, 'lineage:abc123|remote:', ?2, 'planted', 'open', ?3, 1, 1)",
        rusqlite::params![project, source, scoring],
    )
    .unwrap();
}

/// **`AC-P3-28-13`.** `abandoned_with_debt` does not satisfy its own predicate. A project whose
/// **only** open item is `abandoned_with_debt` has that item closed by the next sweep, and its
/// open-item set becomes **empty** — asserted over the item set and not over the layer.
///
/// A predicate that counted its own item would be self-satisfying: the item could never close,
/// and **no abandoned project could ever reach Done** — exactly the proof A2 was made to
/// preserve, re-broken one level down.
#[test]
fn ac_p3_28_13_the_conjunct_does_not_satisfy_itself() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    insert_location(&conn, p);
    set_abandoned(&conn, p, true);
    grant_content_scan(&conn);
    plant_item(&conn, p, "todo_marker", "scored");

    // One scored item of another source, so the conjunct lights and the item opens.
    let tx = conn.transaction().unwrap();
    assert!(abandoned_conjunct(&tx, ProjectId(p)).unwrap());
    evaluate_singletons(&tx, ProjectId(p), 10, &SqliteDebtStore).unwrap();
    tx.commit().unwrap();
    assert_eq!(items_of(&conn, p, "abandoned_with_debt"), 1);

    // The other item is fixed. Now the only open item is `abandoned_with_debt` itself.
    conn.execute(
        "DELETE FROM debt_item WHERE project_id = ?1 AND source = 'todo_marker'",
        [p],
    )
    .unwrap();

    let fixed_tx = conn.transaction().unwrap();
    assert!(
        !abandoned_conjunct(&fixed_tx, ProjectId(p)).unwrap(),
        "the conjunct counted its own item"
    );
    evaluate_singletons(&fixed_tx, ProjectId(p), 20, &SqliteDebtStore).unwrap();
    fixed_tx.commit().unwrap();

    let open: i64 = conn
        .query_row(
            "SELECT count(*) FROM debt_item WHERE project_id = ?1 AND state = 'open'",
            [p],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        open, 0,
        "an abandoned project could never reach an empty set"
    );
}

/// **R122.** The conjunct counts **`scored` items only**. An unfixable advisory is not
/// outstanding work, and saying so is a false accusation of the kind A7 forbids.
#[test]
fn a_shown_only_item_does_not_light_the_conjunct() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    insert_location(&conn, p);
    set_abandoned(&conn, p, true);
    plant_item(&conn, p, "dependency_advisory", "shown_only");

    let tx = conn.transaction().unwrap();
    assert!(
        !abandoned_conjunct(&tx, ProjectId(p)).unwrap(),
        "a shown_only item was counted as outstanding work"
    );
    evaluate_singletons(&tx, ProjectId(p), 10, &SqliteDebtStore).unwrap();
    tx.commit().unwrap();
    assert_eq!(items_of(&conn, p, "abandoned_with_debt"), 0);

    // The same project with one scored item **does** light it.
    grant_content_scan(&conn);
    plant_item(&conn, p, "todo_marker", "scored");
    let scored_tx = conn.transaction().unwrap();
    assert!(abandoned_conjunct(&scored_tx, ProjectId(p)).unwrap());
    evaluate_singletons(&scored_tx, ProjectId(p), 20, &SqliteDebtStore).unwrap();
    scored_tx.commit().unwrap();
    assert_eq!(items_of(&conn, p, "abandoned_with_debt"), 1);
}

/// **Closability is carried by the other conjunct.** The item closes when `condition_signal`
/// leaves `abandoned`, which is an act — and that is what makes this source pass §28.3's
/// no-elapsed-time test, because of its second conjunct rather than in spite of it.
#[test]
fn leaving_the_abandoned_band_closes_the_item() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    insert_location(&conn, p);
    set_abandoned(&conn, p, true);
    grant_content_scan(&conn);
    plant_item(&conn, p, "todo_marker", "scored");

    let tx = conn.transaction().unwrap();
    evaluate_singletons(&tx, ProjectId(p), 10, &SqliteDebtStore).unwrap();
    tx.commit().unwrap();
    assert_eq!(items_of(&conn, p, "abandoned_with_debt"), 1);

    set_abandoned(&conn, p, false);
    let revived_tx = conn.transaction().unwrap();
    assert!(!abandoned_conjunct(&revived_tx, ProjectId(p)).unwrap());
    evaluate_singletons(&revived_tx, ProjectId(p), 20, &SqliteDebtStore).unwrap();
    revived_tx.commit().unwrap();
    assert_eq!(items_of(&conn, p, "abandoned_with_debt"), 0);
}

/// Its item is `shown_only` by registry default, its `basis` is NULL and its `location_id` is
/// NULL: it is derived from other stored observations and observes nothing itself, so paying XP
/// for its closure would be paying for **activity**.
#[test]
fn the_abandoned_item_is_shown_only_and_anchored_nowhere() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    insert_location(&conn, p);
    set_abandoned(&conn, p, true);
    grant_content_scan(&conn);
    plant_item(&conn, p, "todo_marker", "scored");

    let tx = conn.transaction().unwrap();
    evaluate_singletons(&tx, ProjectId(p), 10, &SqliteDebtStore).unwrap();
    tx.commit().unwrap();

    let (scoring, basis, anchor): (String, Option<String>, Option<i64>) = conn
        .query_row(
            "SELECT scoring, basis, last_seen_location_id FROM debt_item
              WHERE project_id = ?1 AND source = 'abandoned_with_debt'",
            [p],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(scoring, "shown_only");
    assert_eq!(
        basis, None,
        "a source that observes nothing invented a basis"
    );
    assert_eq!(anchor, None);

    let sweep_basis: Option<String> = conn
        .query_row(
            "SELECT basis FROM debt_sweep WHERE project_id = ?1 AND source = 'abandoned_with_debt'",
            [p],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(sweep_basis, None);
}

/// **The two halves of the conjunct are two rulings, and only one of them is observable through
/// the default item set.** `abandoned_with_debt` is `shown_only` by registry default, so R122's
/// `scoring = 'scored'` narrowing *already* filters this source's own item and the
/// `source <> 'abandoned_with_debt'` exclusion looks redundant against it.
///
/// **It is not redundant, and this is the case that shows it.** `scoring` is overridable per item
/// by a producer (§32 does exactly that), so an `abandoned_with_debt` item stored `scored` is a
/// representable state — and under it a conjunct without the exclusion is self-satisfying: the
/// item could never close, and **no abandoned project could ever reach Done**.
#[test]
fn the_self_exclusion_holds_even_for_a_scored_abandoned_item() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    insert_location(&conn, p);
    set_abandoned(&conn, p, true);
    plant_item(&conn, p, "abandoned_with_debt", "scored");

    let tx = conn.transaction().unwrap();
    assert!(
        !abandoned_conjunct(&tx, ProjectId(p)).unwrap(),
        "the conjunct counted its own item, so it can never close"
    );
    evaluate_singletons(&tx, ProjectId(p), 10, &SqliteDebtStore).unwrap();
    tx.commit().unwrap();

    let open: i64 = conn
        .query_row(
            "SELECT count(*) FROM debt_item WHERE project_id = ?1 AND state = 'open'",
            [p],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        open, 0,
        "an abandoned project could never reach an empty set"
    );
}

// ---------------------------------------------------------------------------------------------
// §31.9 — *a check that is `na` produces no debt item: unevaluable data never becomes one*
// ---------------------------------------------------------------------------------------------

/// **The arm of a check that is N/A for the project does not run.** It opens nothing on a project
/// whose archetype proposes the check N/A; and on an item already open when the user ruled the
/// check N/A, an observation that would close it closes nothing and pays nothing.
#[test]
fn a_check_that_is_not_applicable_opens_nothing_and_closes_nothing() {
    use codotheca_core::completion::{evaluate_and_write, set_check_na};
    use codotheca_core::debt::singletons::settle_singletons;
    use codotheca_core::protocol::CompletionCheck;

    let (_d, mut conn) = fresh();

    // A docs project, whose archetype proposes `tests` N/A, with no tests and no README.
    let docs = insert_project(&conn, "docs");
    insert_location(&conn, docs);
    conn.execute(
        "UPDATE project SET archetype = 'docs' WHERE id = ?1",
        [docs],
    )
    .unwrap();
    presence(&conn, docs, "absent", "present", "absent");
    let tx = conn.transaction().unwrap();
    evaluate_singletons(&tx, ProjectId(docs), 10, &SqliteDebtStore).unwrap();
    tx.commit().unwrap();
    eprintln!(
        "docs project: missing_readme items {}, missing_tests items {}, missing_tests sweep {:?}",
        items_of(&conn, docs, "missing_readme"),
        items_of(&conn, docs, "missing_tests"),
        sweep_of(&conn, docs, "missing_tests")
    );
    assert_eq!(
        items_of(&conn, docs, "missing_readme"),
        1,
        "the control opened nothing"
    );
    assert_eq!(
        items_of(&conn, docs, "missing_tests"),
        0,
        "an N/A check became a debt item"
    );
    // Not observed, so nothing claims it was: no sweep row speaks for a check that did not run.
    assert_eq!(sweep_of(&conn, docs, "missing_tests"), None);

    // A project with an open `missing_tests` item, which the user then rules N/A.
    let lib = insert_project(&conn, "lib");
    insert_location(&conn, lib);
    conn.execute(
        "UPDATE project SET authored_by_user = 1 WHERE id = ?1",
        [lib],
    )
    .unwrap();
    presence(&conn, lib, "present", "present", "absent");
    let lib_tx = conn.transaction().unwrap();
    evaluate_singletons(&lib_tx, ProjectId(lib), 10, &SqliteDebtStore).unwrap();
    evaluate_and_write(&lib_tx, ProjectId(lib), 10).unwrap();
    set_check_na(
        &lib_tx,
        ProjectId(lib),
        CompletionCheck::Tests,
        Some(true),
        10,
    )
    .unwrap();
    lib_tx.commit().unwrap();
    let ruled: Option<i64> = conn
        .query_row(
            "SELECT user_na FROM project_check WHERE project_id = ?1 AND check_key = 'tests'",
            [lib],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ruled, Some(1), "the ruling did not land");
    assert_eq!(items_of(&conn, lib, "missing_tests"), 1);

    // Tests now exist: an observation that would close the item — if the arm ran.
    presence(&conn, lib, "present", "present", "present");
    let xp = |db: &rusqlite::Connection| -> i64 {
        db.query_row("SELECT count(*) FROM xp_events", [], |r| r.get(0))
            .unwrap()
    };
    let xp_before = xp(&conn);
    let settle_tx = conn.transaction().unwrap();
    let effect = settle_singletons(&settle_tx, ProjectId(lib), 20, 0, &SqliteDebtStore).unwrap();
    settle_tx.commit().unwrap();
    eprintln!(
        "lib project after an N/A ruling: missing_tests items {}, closed {:?}, xp_events {} -> {}",
        items_of(&conn, lib, "missing_tests"),
        effect.closed,
        xp_before,
        xp(&conn)
    );
    assert_eq!(
        items_of(&conn, lib, "missing_tests"),
        1,
        "an N/A ruling closed the item"
    );
    assert_eq!(xp(&conn), xp_before, "an N/A ruling paid");
}

/// §30.9 and R128/F8 — **`todo_marker` off is not swept**, whether its switch is off or §29.8's
/// grant is missing. An ungranted run reads no blob, so its empty occurrence list is not an
/// observation of an empty set: sweeping it would close every item as fixed and pay for it.
#[test]
fn a_switched_off_or_ungranted_todo_marker_is_not_swept() {
    use codotheca_core::health::switches::write_switches;
    use codotheca_core::protocol::{DebtSource, HealthCheckSwitch};

    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let loc = insert_location(&conn, p);
    content_scan(&conn, p, true);
    let store = SqliteDebtStore;
    let toggle = |db: &rusqlite::Connection, enabled: bool| {
        let tx = db.unchecked_transaction().unwrap();
        write_switches(
            &tx,
            &[HealthCheckSwitch {
                check: DebtSource::TodoMarker,
                enabled,
            }],
        )
        .unwrap();
        tx.commit().unwrap();
    };

    // The control: switched on and granted, the run sweeps and opens.
    let one = vec![occurrence("a.rs", 4, "x")];
    let tx = conn.transaction().unwrap();
    build_items(&tx, ProjectId(p), Some(loc), gates(true), &one, 10, &store).unwrap();
    tx.commit().unwrap();
    assert_eq!(items(&conn, p).len(), 1, "the control opened nothing");
    assert!(sweep_of(&conn, p, "todo_marker").is_some());

    // Switched off: a run finding nothing would close the item — if it swept.
    toggle(&conn, false);
    let off_tx = conn.transaction().unwrap();
    let off = build_items(
        &off_tx,
        ProjectId(p),
        Some(loc),
        gates(true),
        &[],
        20,
        &store,
    )
    .unwrap();
    off_tx.commit().unwrap();
    eprintln!(
        "switched off: sweep {:?}, items {}, closed {}",
        sweep_of(&conn, p, "todo_marker"),
        items(&conn, p).len(),
        off.closed.len()
    );
    assert_eq!(
        sweep_of(&conn, p, "todo_marker"),
        None,
        "a switched-off source was swept"
    );
    assert_eq!(items(&conn, p).len(), 1, "a switched-off source closed");

    // Back on, but ungranted: the same, for R128/F8's second cause of `off`.
    toggle(&conn, true);
    let ungranted = ContentGates {
        granted: false,
        ..gates(true)
    };
    let ungranted_tx = conn.transaction().unwrap();
    let ungranted_effect = build_items(
        &ungranted_tx,
        ProjectId(p),
        Some(loc),
        ungranted,
        &[],
        30,
        &store,
    )
    .unwrap();
    ungranted_tx.commit().unwrap();
    eprintln!(
        "ungranted: sweep {:?}, items {}, closed {}",
        sweep_of(&conn, p, "todo_marker"),
        items(&conn, p).len(),
        ungranted_effect.closed.len()
    );
    assert_eq!(
        sweep_of(&conn, p, "todo_marker"),
        None,
        "an ungranted source was swept"
    );
    assert_eq!(items(&conn, p).len(), 1, "an ungranted run closed an item");
}

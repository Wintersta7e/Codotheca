//! §28.3 and §28.5 — the two-state lifecycle, the sweep record, and the four rules that make
//! zero mean something.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::debt::identity::DebtKey;
use codotheca_core::debt::store::StoredItem;
use codotheca_core::debt::sweep::{
    comparable, latest_sweep, may_close, outcome_at_root, root_is_observable, upsert_sweep,
    SweepObservation,
};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::protocol::{
    DebtItemState, DebtScoring, DebtSource, DebtSweepOutcome, LocationId, ObservationBasis,
    ProjectId,
};

const SUBJECT: &str = "lineage:abc123|remote:";

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

fn insert_location(conn: &rusqlite::Connection, project: i64, presence: &str) -> i64 {
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind)
         VALUES (?1, 'linux', x'2f61', x'2f61', '/a', 'store', ?2, 'worktree')",
        rusqlite::params![project, presence],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// A stored item shaped only by what the closure rules read.
fn item_at(
    source: DebtSource,
    location: Option<LocationId>,
    basis: Option<ObservationBasis>,
) -> StoredItem {
    StoredItem {
        id: 1,
        key: DebtKey::singleton(SUBJECT, source),
        state: DebtItemState::Open,
        scoring: DebtScoring::Scored,
        last_seen_location_id: location,
        basis,
        path_display: None,
        line: None,
        column: None,
        salient_text: None,
        first_seen_at: 1,
        last_seen_at: 1,
    }
}

fn sweep_at(
    project: i64,
    source: DebtSource,
    outcome: DebtSweepOutcome,
    location: Option<LocationId>,
    basis: Option<ObservationBasis>,
) -> SweepObservation {
    SweepObservation {
        project: ProjectId(project),
        source,
        outcome,
        location,
        generation: None,
        basis,
        item_count: match outcome {
            DebtSweepOutcome::Complete | DebtSweepOutcome::Partial => Some(0),
            _ => None,
        },
        observed_at: 10,
    }
}

// ---------------------------------------------------------------------------------------------
// Rule 1 — a closure needs a `complete` sweep, at the item's anchor, on the item's basis
// ---------------------------------------------------------------------------------------------

#[test]
fn a_sweep_at_another_anchor_closes_nothing() {
    let a = Some(LocationId(1));
    let b = Some(LocationId(2));
    let item = item_at(DebtSource::MissingReadme, a, Some(ObservationBasis::Head));
    let obs = sweep_at(
        1,
        DebtSource::MissingReadme,
        DebtSweepOutcome::Complete,
        b,
        Some(ObservationBasis::Head),
    );

    assert!(
        !comparable(&item, &obs),
        "rule 1's anchor half did not hold"
    );
    assert!(!may_close(&item, &obs));
}

/// **`AC-P3-28-12`, A9.** A worktree-basis observation and a HEAD-basis observation are not
/// comparable: one reads what is written down and the other reads what is on disk.
#[test]
fn ac_p3_28_12_a_sweep_on_another_basis_closes_nothing() {
    let anchor = Some(LocationId(1));
    let item = item_at(DebtSource::TodoMarker, anchor, Some(ObservationBasis::Head));
    let obs = sweep_at(
        1,
        DebtSource::TodoMarker,
        DebtSweepOutcome::Complete,
        anchor,
        Some(ObservationBasis::Worktree),
    );

    assert!(!comparable(&item, &obs), "two bases were treated as one");
    assert!(!may_close(&item, &obs));

    // And the same sweep on the item's own basis does close it, so the refusal above is the
    // basis and not something else about the pair.
    let same = sweep_at(
        1,
        DebtSource::TodoMarker,
        DebtSweepOutcome::Complete,
        anchor,
        Some(ObservationBasis::Head),
    );
    assert!(may_close(&item, &same));
}

/// What the `IS`-comparison buys: a NULL anchor matches a NULL anchor. `abandoned_with_debt`
/// carries neither an anchor nor a basis, and under SQL's `=` it could never close.
#[test]
fn a_null_anchored_item_is_closed_by_a_null_anchored_sweep() {
    let item = item_at(DebtSource::AbandonedWithDebt, None, None);
    let obs = sweep_at(
        1,
        DebtSource::AbandonedWithDebt,
        DebtSweepOutcome::Complete,
        None,
        None,
    );
    assert!(comparable(&item, &obs));
    assert!(may_close(&item, &obs));
}

// ---------------------------------------------------------------------------------------------
// Rule 2 — a `partial` sweep may open and may never close
// ---------------------------------------------------------------------------------------------

/// **`AC-P3-28-2`.** An item a partial sweep did not reach looks exactly like an item that is
/// gone, and the difference is not recoverable afterwards.
#[test]
fn ac_p3_28_2_a_partial_sweep_closes_nothing() {
    let anchor = Some(LocationId(1));
    let basis = Some(ObservationBasis::Head);
    let item = item_at(DebtSource::TodoMarker, anchor, basis);

    for outcome in [
        DebtSweepOutcome::Partial,
        DebtSweepOutcome::Failed,
        DebtSweepOutcome::Unobservable,
        DebtSweepOutcome::SkippedReference,
        DebtSweepOutcome::SkippedSuppressed,
    ] {
        let obs = sweep_at(1, DebtSource::TodoMarker, outcome, anchor, basis);
        assert!(
            !may_close(&item, &obs),
            "{outcome:?} must not close an item"
        );
        // …and it is rule 2 refusing, not rule 1: the pair is comparable.
        assert!(comparable(&item, &obs));
    }
}

// ---------------------------------------------------------------------------------------------
// Rules 3 and 4 — the freeze, and the uninstall hole
// ---------------------------------------------------------------------------------------------

#[test]
fn an_offline_root_is_not_a_sweep_with_zero_results() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let offline = LocationId(insert_location(&conn, p, "offline"));
    let tx = conn.transaction().unwrap();

    assert!(!root_is_observable(&tx, offline).unwrap());
    assert_eq!(
        outcome_at_root(&tx, Some(offline), DebtSweepOutcome::Complete).unwrap(),
        DebtSweepOutcome::Unobservable,
        "an offline root reported complete with zero items"
    );
}

/// **The uninstall hole.** `locations.uninstall` removes the bytes and keeps the row: `presence`
/// still reads `present` and only `removed_at` says otherwise. A guard reading `presence` alone
/// finds a readable-looking absence, reports `complete` with zero items, closes every item and
/// pays for it.
#[test]
fn a_removed_root_is_unobservable_while_presence_still_reads_present() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let loc = LocationId(insert_location(&conn, p, "present"));
    conn.execute("UPDATE location SET removed_at = 99 WHERE id = ?1", [loc.0])
        .unwrap();

    let presence: String = conn
        .query_row(
            "SELECT presence FROM location WHERE id = ?1",
            [loc.0],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(presence, "present", "the fixture is not the hole it claims");

    let tx = conn.transaction().unwrap();
    assert!(!root_is_observable(&tx, loc).unwrap());
    assert_eq!(
        outcome_at_root(&tx, Some(loc), DebtSweepOutcome::Complete).unwrap(),
        DebtSweepOutcome::Unobservable,
    );
}

#[test]
fn a_present_root_with_no_items_is_complete_with_a_count_of_zero() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let loc = LocationId(insert_location(&conn, p, "present"));
    let tx = conn.transaction().unwrap();

    assert!(root_is_observable(&tx, loc).unwrap());
    assert_eq!(
        outcome_at_root(&tx, Some(loc), DebtSweepOutcome::Complete).unwrap(),
        DebtSweepOutcome::Complete,
    );
}

/// A sweep with no anchor has no root to freeze against, and inventing one would be a lie.
#[test]
fn a_sweep_with_no_anchor_keeps_the_outcome_it_proposed() {
    let (_d, mut conn) = fresh();
    let tx = conn.transaction().unwrap();
    assert_eq!(
        outcome_at_root(&tx, None, DebtSweepOutcome::Complete).unwrap(),
        DebtSweepOutcome::Complete,
    );
}

// ---------------------------------------------------------------------------------------------
// The record itself
// ---------------------------------------------------------------------------------------------

/// One row per `(project, source)`, upserted — never a history. And **never observed** is `None`
/// rather than an outcome, which is what makes *not computed* readable as itself.
#[test]
fn the_sweep_row_is_upserted_and_absence_is_not_an_outcome() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let loc = LocationId(insert_location(&conn, p, "present"));
    let tx = conn.transaction().unwrap();

    assert_eq!(
        latest_sweep(&tx, ProjectId(p), DebtSource::TodoMarker).unwrap(),
        None,
        "a source nobody swept must read as never observed"
    );

    let first = sweep_at(
        p,
        DebtSource::TodoMarker,
        DebtSweepOutcome::Complete,
        Some(loc),
        Some(ObservationBasis::Head),
    );
    upsert_sweep(&tx, &first).unwrap();
    assert_eq!(
        latest_sweep(&tx, ProjectId(p), DebtSource::TodoMarker)
            .unwrap()
            .as_ref(),
        Some(&first),
    );

    let mut second = first.clone();
    second.outcome = DebtSweepOutcome::Partial;
    second.item_count = Some(3);
    second.observed_at = 20;
    upsert_sweep(&tx, &second).unwrap();

    let rows: i64 = tx
        .query_row(
            "SELECT count(*) FROM debt_sweep WHERE project_id = ?1 AND source = 'todo_marker'",
            [p],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 1, "a sweep history was kept, and nothing reads one");
    assert_eq!(
        latest_sweep(&tx, ProjectId(p), DebtSource::TodoMarker).unwrap(),
        Some(second),
    );
}

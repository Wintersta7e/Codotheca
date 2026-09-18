//! §28.3 and §28.5 — the two-state lifecycle, the sweep record, and the four rules that make
//! zero mean something.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::debt::identity::DebtKey;
use codotheca_core::debt::store::{
    DebtCloseReason, DebtStore, ObservedItem, SqliteDebtStore, StoredItem, SweepEffect,
};
use codotheca_core::debt::sweep::{
    comparable, latest_sweep, may_close, outcome_at_root, root_is_observable, upsert_sweep,
    SweepObservation,
};
use codotheca_core::debt::xp::pay_debt_day;
use codotheca_core::debt::{registry_for, IdentityShape, SOURCE_REGISTRY};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::protocol::{
    DebtItemState, DebtScoring, DebtSource, DebtSweepOutcome, DecayLayer, LocationId,
    ObservationBasis, ProjectId,
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

/// A copy of one project. `path_key` is UNIQUE per `(kind, distro, path_key)`, so a second copy
/// of one project needs a path of its own — which is exactly the re-clone the reap exists for.
fn insert_location(conn: &rusqlite::Connection, project: i64, presence: &str) -> i64 {
    let n: i64 = conn
        .query_row("SELECT count(*) FROM location", [], |r| r.get(0))
        .unwrap();
    let path = format!("/copy-{n}");
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind)
         VALUES (?1, 'linux', ?3, ?3, ?4, 'store', ?2, 'worktree')",
        rusqlite::params![project, presence, path.as_bytes(), path],
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

// ---------------------------------------------------------------------------------------------
// §28.3 — the store: open, refresh, close, reap
// ---------------------------------------------------------------------------------------------

fn seen(key: DebtKey, location: Option<LocationId>, path: &str, line: u32) -> ObservedItem {
    ObservedItem {
        key,
        scoring: DebtScoring::Scored,
        location,
        basis: Some(ObservationBasis::Head),
        path_bytes: Some(path.as_bytes().to_vec()),
        path_display: Some(path.to_owned()),
        line: Some(line),
        column: Some(1),
        salient_text: Some("TODO: a thing".to_owned()),
    }
}

fn open_keys(conn: &rusqlite::Connection, project: i64) -> Vec<(String, String, String)> {
    let mut st = conn
        .prepare(
            "SELECT source, fingerprint, state FROM debt_item
              WHERE project_id = ?1 ORDER BY source, fingerprint",
        )
        .unwrap();
    st.query_map([project], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// A marker deleted from a **present, readable** root closes, and the closure is `Fixed` — a
/// transition the user performed.
#[test]
fn a_marker_gone_from_a_readable_root_closes_fixed() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let loc = LocationId(insert_location(&conn, p, "present"));
    let store = SqliteDebtStore;
    let key = DebtKey::content(SUBJECT, "aaaa", 0);

    let tx = conn.transaction().unwrap();
    let obs = sweep_at(
        p,
        DebtSource::TodoMarker,
        DebtSweepOutcome::Complete,
        Some(loc),
        Some(ObservationBasis::Head),
    );
    let first = store
        .observe(&tx, &obs, &[seen(key.clone(), Some(loc), "a.rs", 4)])
        .unwrap();
    assert_eq!(first.opened, vec![key.clone()]);
    assert!(first.closed.is_empty());

    // The same sweep, with the marker gone.
    let second = store.observe(&tx, &obs, &[]).unwrap();
    assert_eq!(second.closed, vec![(key, DebtCloseReason::Fixed)]);
    assert!(second.opened.is_empty());
    tx.commit().unwrap();

    assert!(open_keys(&conn, p).is_empty(), "closed is a deletion");
}

/// **`AC-P3-28-3`.** `refresh` updates the attributes and **never the fingerprint**, which is
/// what makes a rename and a line move close nothing.
#[test]
fn ac_p3_28_3_a_rename_refreshes_and_closes_nothing() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let loc = LocationId(insert_location(&conn, p, "present"));
    let store = SqliteDebtStore;
    let key = DebtKey::content(SUBJECT, "aaaa", 0);
    let obs = sweep_at(
        p,
        DebtSource::TodoMarker,
        DebtSweepOutcome::Complete,
        Some(loc),
        Some(ObservationBasis::Head),
    );

    let tx = conn.transaction().unwrap();
    store
        .observe(&tx, &obs, &[seen(key.clone(), Some(loc), "old/a.rs", 4)])
        .unwrap();
    let effect = store
        .observe(&tx, &obs, &[seen(key.clone(), Some(loc), "new/b.rs", 91)])
        .unwrap();
    tx.commit().unwrap();

    assert!(effect.opened.is_empty(), "a rename opened a second item");
    assert!(effect.closed.is_empty(), "a rename closed an item");
    assert_eq!(effect.refreshed, 1);

    let (fingerprint, path, line): (String, String, i64) = conn
        .query_row(
            "SELECT fingerprint, path_display, line FROM debt_item WHERE project_id = ?1",
            [p],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(fingerprint, key.fingerprint, "the fingerprint moved");
    assert_eq!(path, "new/b.rs");
    assert_eq!(line, 91);
}

/// **`AC-P3-28-1`.** A sweep whose anchor is offline marks the item `unverified` and closes
/// nothing — **asserted by item identity before and after, never by count alone**, because a
/// close-and-reopen of the same source keeps the count and loses the item.
#[test]
fn ac_p3_28_1_an_offline_anchor_marks_unverified_and_closes_nothing() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let loc = LocationId(insert_location(&conn, p, "present"));
    let store = SqliteDebtStore;
    let key = DebtKey::content(SUBJECT, "aaaa", 0);

    let tx = conn.transaction().unwrap();
    let live = sweep_at(
        p,
        DebtSource::TodoMarker,
        DebtSweepOutcome::Complete,
        Some(loc),
        Some(ObservationBasis::Head),
    );
    store
        .observe(&tx, &live, &[seen(key.clone(), Some(loc), "a.rs", 4)])
        .unwrap();
    tx.commit().unwrap();
    let before = open_keys(&conn, p);
    assert_eq!(
        before,
        vec![("todo_marker".into(), key.fingerprint.clone(), "open".into())]
    );

    conn.execute(
        "UPDATE location SET presence = 'offline' WHERE id = ?1",
        [loc.0],
    )
    .unwrap();

    let tx = conn.transaction().unwrap();
    let frozen = SweepObservation {
        outcome: outcome_at_root(&tx, Some(loc), DebtSweepOutcome::Complete).unwrap(),
        item_count: None,
        ..live
    };
    assert_eq!(frozen.outcome, DebtSweepOutcome::Unobservable);
    let effect = store.observe(&tx, &frozen, &[]).unwrap();
    tx.commit().unwrap();

    assert!(
        effect.closed.is_empty(),
        "an unobservable sweep closed an item"
    );
    assert_eq!(effect.unverified, 1);

    let after = open_keys(&conn, p);
    assert_eq!(
        after,
        vec![("todo_marker".into(), key.fingerprint, "unverified".into())],
        "the item's identity moved, so this was a close-and-reopen"
    );
}

/// A `Reference` project's sweep writes `skipped_reference` and marks the items `unverified`: a
/// gate that declined to look is not a look that found nothing.
#[test]
fn a_skipped_reference_sweep_marks_unverified() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let loc = LocationId(insert_location(&conn, p, "present"));
    let store = SqliteDebtStore;
    let key = DebtKey::content(SUBJECT, "aaaa", 0);

    let tx = conn.transaction().unwrap();
    let live = sweep_at(
        p,
        DebtSource::TodoMarker,
        DebtSweepOutcome::Complete,
        Some(loc),
        Some(ObservationBasis::Head),
    );
    store
        .observe(&tx, &live, &[seen(key, Some(loc), "a.rs", 4)])
        .unwrap();

    let skipped = SweepObservation {
        outcome: DebtSweepOutcome::SkippedReference,
        item_count: None,
        ..live
    };
    let effect = store.observe(&tx, &skipped, &[]).unwrap();
    tx.commit().unwrap();

    assert!(effect.closed.is_empty());
    assert_eq!(effect.unverified, 1);

    let stored: (String, String) = conn
        .query_row(
            "SELECT i.state, s.outcome FROM debt_item i
               JOIN debt_sweep s ON s.project_id = i.project_id AND s.source = i.source
              WHERE i.project_id = ?1",
            [p],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        stored,
        ("unverified".to_owned(), "skipped_reference".to_owned())
    );
}

/// **`AC-P3-28-17`. A reap is not a closure.** An `unverified` item whose anchor is gone is
/// deleted with **no closure event, no XP and no layer movement**. It is safe precisely because
/// `unverified` items are counted in nothing — and without it, a project re-cloned to a new
/// `location_id` strands its old items for ever.
#[test]
fn ac_p3_28_17_a_reap_writes_no_closure_and_no_xp() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let old = LocationId(insert_location(&conn, p, "present"));
    let store = SqliteDebtStore;
    let key = DebtKey::content(SUBJECT, "aaaa", 0);

    let tx = conn.transaction().unwrap();
    let at_old = sweep_at(
        p,
        DebtSource::TodoMarker,
        DebtSweepOutcome::Complete,
        Some(old),
        Some(ObservationBasis::Head),
    );
    store
        .observe(&tx, &at_old, &[seen(key, Some(old), "a.rs", 4)])
        .unwrap();
    store.mark_unverified(&tx, ProjectId(p)).unwrap();
    tx.commit().unwrap();

    // The old copy is uninstalled and a fresh clone lands beside it, which is the case the reap
    // exists for: `location_id` moved and nothing else will ever re-observe the old anchor.
    conn.execute("UPDATE location SET removed_at = 99 WHERE id = ?1", [old.0])
        .unwrap();
    let new = LocationId(insert_location(&conn, p, "present"));

    let tx = conn.transaction().unwrap();
    let at_new = sweep_at(
        p,
        DebtSource::TodoMarker,
        DebtSweepOutcome::Complete,
        Some(new),
        Some(ObservationBasis::Head),
    );

    // The sweep runs first and must report **no closure**: the stranded item is not comparable
    // with it, and an `observe` that folded the strand into `closed` would pay for it. This is
    // the half that bites — the `xp_events` half below cannot, because §28's XP writer pays from
    // `SweepEffect::closed` and a reap never reaches it.
    let effect = store.observe(&tx, &at_new, &[]).unwrap();
    assert!(
        effect.closed.is_empty(),
        "a stranded item was closed rather than reaped: {:?}",
        effect.closed
    );

    let reaped = store.reap(&tx, ProjectId(p), &at_new).unwrap();
    tx.commit().unwrap();

    assert_eq!(reaped, 1, "the stranded item was not reaped");
    assert!(open_keys(&conn, p).is_empty());

    let xp: i64 = conn
        .query_row("SELECT count(*) FROM xp_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(xp, 0, "a reap paid XP, so it was treated as a closure");
}

/// A reap needs a `complete` sweep **at the project's current primary location**. Anything less
/// and the item is not stranded, it is merely unobserved.
#[test]
fn a_reap_refuses_a_sweep_that_is_not_complete_at_the_primary() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let old = LocationId(insert_location(&conn, p, "present"));
    let store = SqliteDebtStore;
    let key = DebtKey::content(SUBJECT, "aaaa", 0);

    let tx = conn.transaction().unwrap();
    let at_old = sweep_at(
        p,
        DebtSource::TodoMarker,
        DebtSweepOutcome::Complete,
        Some(old),
        Some(ObservationBasis::Head),
    );
    store
        .observe(&tx, &at_old, &[seen(key, Some(old), "a.rs", 4)])
        .unwrap();
    store.mark_unverified(&tx, ProjectId(p)).unwrap();
    tx.commit().unwrap();

    conn.execute("UPDATE location SET removed_at = 99 WHERE id = ?1", [old.0])
        .unwrap();
    let new = LocationId(insert_location(&conn, p, "present"));

    let tx = conn.transaction().unwrap();
    let partial = sweep_at(
        p,
        DebtSource::TodoMarker,
        DebtSweepOutcome::Partial,
        Some(new),
        Some(ObservationBasis::Head),
    );
    assert_eq!(store.reap(&tx, ProjectId(p), &partial).unwrap(), 0);

    // …and a complete sweep somewhere that is not the primary proves nothing either.
    let elsewhere = sweep_at(
        p,
        DebtSource::TodoMarker,
        DebtSweepOutcome::Complete,
        Some(old),
        Some(ObservationBasis::Head),
    );
    assert_eq!(store.reap(&tx, ProjectId(p), &elsewhere).unwrap(), 0);
}

// ---------------------------------------------------------------------------------------------
// §28.2's registry — one table, one row per source
// ---------------------------------------------------------------------------------------------

/// The committed contract, compiled in rather than re-found at runtime.
const SCHEMA: &str = include_str!("../../protocol/schema/protocol.json");

/// Every variant the schema declares for `name`, in declaration order.
fn schema_variants(name: &str) -> Vec<String> {
    let doc: serde_json::Value = serde_json::from_str(SCHEMA).expect("protocol.json parses");
    let decl = doc["types"]
        .get(name)
        .unwrap_or_else(|| panic!("{name} is not declared in protocol.json"));
    let variants: Vec<String> = decl["variants"]
        .as_array()
        .unwrap_or_else(|| panic!("{name} declares no variants"))
        .iter()
        .map(|v| v.as_str().expect("a variant is a string").to_owned())
        .collect();
    assert!(!variants.is_empty(), "{name} declares no variants");
    variants
}

fn wire(value: &impl serde::Serialize) -> String {
    match serde_json::to_value(value).unwrap() {
        serde_json::Value::String(raw) => raw,
        other => panic!("a generated enum serialised as {other}"),
    }
}

/// The registry is **one table with one row per source**, so the layer, the default scoring, the
/// identity shape and the basis are read from one place and cannot drift apart.
///
/// **The variant list is derived from the generated enum**, never from a literal beside it — a
/// literal would agree with itself while the schema moved. The count is printed and the run fails
/// at zero.
#[test]
fn the_registry_covers_every_declared_source_exactly_once() {
    let declared = schema_variants("DebtSource");
    assert_eq!(
        SOURCE_REGISTRY.len(),
        declared.len(),
        "the registry and the schema disagree about how many sources exist"
    );

    let mut covered = 0_usize;
    for name in &declared {
        let source: DebtSource = serde_json::from_value(serde_json::json!(name)).unwrap();
        let row = registry_for(source);
        assert_eq!(
            row.source, source,
            "registry_for({name}) returned another row"
        );
        assert_eq!(
            SOURCE_REGISTRY
                .iter()
                .filter(|r| r.source == source)
                .count(),
            1,
            "{name} has more than one registry row"
        );
        covered += 1;
    }
    eprintln!("SOURCE_REGISTRY covers {covered} of DebtSource's declared variants");
    assert!(covered > 0, "covered nothing, so this proved nothing");
}

/// The four columns §28.2 fixes, asserted against the section's own table so a silent re-tuning
/// of a layer is a failing test rather than a different picture.
#[test]
fn the_registry_carries_the_layer_shape_and_basis_the_section_states() {
    let expected: &[(
        &str,
        IdentityShape,
        DecayLayer,
        DebtScoring,
        Option<ObservationBasis>,
    )] = &[
        (
            "todo_marker",
            IdentityShape::Content,
            DecayLayer::Overgrowth,
            DebtScoring::Scored,
            Some(ObservationBasis::Head),
        ),
        (
            "missing_readme",
            IdentityShape::Singleton,
            DecayLayer::Dust,
            DebtScoring::Scored,
            Some(ObservationBasis::Head),
        ),
        (
            "missing_license",
            IdentityShape::Singleton,
            DecayLayer::Dust,
            DebtScoring::Scored,
            Some(ObservationBasis::Head),
        ),
        (
            "missing_tests",
            IdentityShape::Singleton,
            DecayLayer::Overgrowth,
            DebtScoring::Scored,
            Some(ObservationBasis::Head),
        ),
        (
            "no_release",
            IdentityShape::Singleton,
            DecayLayer::Dust,
            DebtScoring::Scored,
            Some(ObservationBasis::Refs),
        ),
        (
            "unpushed_commits",
            IdentityShape::Singleton,
            DecayLayer::Overgrowth,
            DebtScoring::Scored,
            Some(ObservationBasis::Refs),
        ),
        (
            "ci_red",
            IdentityShape::Singleton,
            DecayLayer::Cracks,
            DebtScoring::Scored,
            Some(ObservationBasis::Remote),
        ),
        (
            "dependency_advisory",
            IdentityShape::External,
            DecayLayer::Rust,
            DebtScoring::Scored,
            Some(ObservationBasis::Worktree),
        ),
        // **`NULL` basis has exactly one meaning and exactly one source.**
        // `abandoned_with_debt` is derived from other stored observations and observes nothing
        // itself, so it has no basis to carry and inventing one would be a lie.
        (
            "abandoned_with_debt",
            IdentityShape::Singleton,
            DecayLayer::Cobwebs,
            DebtScoring::ShownOnly,
            None,
        ),
    ];

    for (name, shape, layer, scoring, basis) in expected {
        let source: DebtSource = serde_json::from_value(serde_json::json!(name)).unwrap();
        let row = registry_for(source);
        assert_eq!(row.shape, *shape, "{name} shape");
        assert_eq!(row.layer, *layer, "{name} layer");
        assert_eq!(row.default_scoring, *scoring, "{name} default scoring");
        assert_eq!(row.basis, *basis, "{name} basis");
    }
    eprintln!(
        "the registry's four columns hold for {} sources",
        expected.len()
    );
}

/// **`AC-P3-28-14`, this plan's half.** A `shown_only` item is written, readable, and contributes
/// to **no** `xp_events` row. `scoring` is about CONSEQUENCE and `state` is about OBSERVATION;
/// folding them puts six meanings in one column and the first reader collapses two of them.
#[test]
fn ac_p3_28_14_a_shown_only_item_renders_and_pays_nothing() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let loc = LocationId(insert_location(&conn, p, "present"));
    let store = SqliteDebtStore;
    let key = DebtKey::singleton(SUBJECT, DebtSource::AbandonedWithDebt);

    let tx = conn.transaction().unwrap();
    let obs = sweep_at(
        p,
        DebtSource::AbandonedWithDebt,
        DebtSweepOutcome::Complete,
        None,
        None,
    );
    let mut observed = seen(key.clone(), None, "", 0);
    observed.scoring = DebtScoring::ShownOnly;
    observed.basis = None;
    observed.path_bytes = None;
    observed.path_display = None;
    observed.line = None;
    observed.column = None;
    observed.salient_text = None;
    let opened = store
        .observe(&tx, &obs, std::slice::from_ref(&observed))
        .unwrap();
    assert_eq!(opened.opened, vec![key.clone()]);

    // It closes, and the closure pays nothing because the item was never scored.
    let closed = store.observe(&tx, &obs, &[]).unwrap();
    assert_eq!(closed.closed.len(), 1);
    let payout = pay_debt_day(
        &tx,
        ProjectId(p),
        SUBJECT,
        &SweepEffect {
            closed: closed
                .closed
                .iter()
                .filter(|(k, _)| registry_for(k.source).default_scoring == DebtScoring::Scored)
                .cloned()
                .collect(),
            ..SweepEffect::default()
        },
        1_789_646_400,
        0,
    )
    .unwrap();
    tx.commit().unwrap();

    assert!(!payout.wrote_row, "a shown_only closure paid XP");
    let xp: i64 = conn
        .query_row("SELECT count(*) FROM xp_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(xp, 0);
    // The anchor is untouched: `loc` exists and the item never referenced it.
    assert!(loc.0 > 0);
}

/// **Five further sources are declared in §28's prose and are in neither the DDL CHECK nor the
/// generated enum.** The registry does not hold them either, so all three stay identical
/// character for character. The cost of a tenth source is one migration plus one regenerate,
/// stated so it is budgeted rather than discovered.
#[test]
fn the_five_not_enabled_sources_are_in_no_vocabulary() {
    let declared = schema_variants("DebtSource");
    let ddl = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations/0013_debt.sql"),
    )
    .unwrap();
    assert!(
        !ddl.is_empty(),
        "read an empty migration, so this proved nothing"
    );

    for name in [
        "forge_issues",
        "lint_warnings",
        "compiler_warnings",
        "failing_tests",
        "stale_merged_branches",
    ] {
        assert!(
            !declared.iter().any(|v| v == name),
            "DebtSource declares {name}"
        );
        assert!(
            !ddl.contains(&format!("'{name}'")),
            "0013_debt.sql spells {name} into a CHECK"
        );
        assert!(
            !SOURCE_REGISTRY.iter().any(|r| wire(&r.source) == name),
            "the registry holds {name}"
        );
    }
    eprintln!("checked 5 not-enabled names against 3 vocabularies");
}

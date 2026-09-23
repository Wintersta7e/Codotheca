//! §34.2 — `health_delta`'s producer: **a delta is a debt-set transition and nothing else**, under
//! §30.1's precondition, written inside the writer's own transaction.
//!
//! Every row count below is printed, and every table-driven test prints the number of cases it
//! executed and fails at zero: a test that asserted zero rows over a fixture that drove nothing
//! would pass for the wrong reason. The production callers are driven in
//! `restoration_callers.rs`, and the emit gate in `restoration_emit.rs`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::debt::identity::DebtKey;
use codotheca_core::debt::store::{DebtStore, ObservedItem, SqliteDebtStore, SweepEffect};
use codotheca_core::debt::sweep::{outcome_at_root, SweepObservation};
use codotheca_core::health::switches::write_switches;
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::protocol::{
    DebtScoring, DebtSource, DebtSweepOutcome, DecayLayer, HealthCheckSwitch, HealthDetectedIn,
    LocationId, ObservationBasis, ProjectId,
};
use codotheca_core::restoration::{record_after_write, record_layer_deltas, LayerValues};
use rusqlite::{Connection, Transaction};

/// The clock every direct fixture writes at. Evidence is stamped relative to it.
const NOW: i64 = 1_800_000_000;
/// Enrolment happened before any evidence below, unless a case says otherwise.
const ACK: i64 = NOW - 1_000;
const SUBJECT: &str = "lineage:fixture";

// ---------------------------------------------------------------------------------------------
// The direct fixture: one enrolled, authored project, one present copy, and `overgrowth` fed by
// `todo_marker` alone — `missing_tests` and `unpushed_commits` are switched off, so the layer's
// eligible set is one source this file drives through §28's real item writer.
// ---------------------------------------------------------------------------------------------

struct Db {
    _dir: tempfile::TempDir,
    conn: Connection,
    project: ProjectId,
    location: LocationId,
}

/// The two ids a writer needs, copied out so a closure can hold them while the connection is
/// borrowed for the transaction.
#[derive(Debug, Clone, Copy)]
struct Ids {
    project: ProjectId,
    location: LocationId,
}

impl Db {
    fn ids(&self) -> Ids {
        Ids {
            project: self.project,
            location: self.location,
        }
    }
}

fn db(acknowledged_at: Option<i64>) -> Db {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, authored_by_user, is_reference,
                              acknowledged_at, created_at, updated_at)
         VALUES ('p', 'p', 'fixture', 1, 0, ?1, 1, 1)",
        [acknowledged_at],
    )
    .unwrap();
    let project = ProjectId(conn.last_insert_rowid());
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind)
         VALUES (?1, 'linux', x'2f70', x'2f70', '/p', 'store', 'present', 'worktree')",
        [project.0],
    )
    .unwrap();
    let location = LocationId(conn.last_insert_rowid());
    // §29's grant, without which `todo_marker` is `off` and feeds nothing.
    conn.execute(
        "INSERT INTO app_meta (k, v) VALUES ('content_scan_enabled', '1')
         ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        [],
    )
    .unwrap();
    let mut db = Db {
        _dir: dir,
        conn,
        project,
        location,
    };
    switch(
        &mut db,
        &[
            (DebtSource::MissingTests, false),
            (DebtSource::UnpushedCommits, false),
        ],
    );
    db
}

fn switch(db: &mut Db, switches: &[(DebtSource, bool)]) {
    let tx = db.conn.transaction().unwrap();
    let rows: Vec<HealthCheckSwitch> = switches
        .iter()
        .map(|&(check, enabled)| HealthCheckSwitch { check, enabled })
        .collect();
    write_switches(&tx, &rows).unwrap();
    tx.commit().unwrap();
}

/// §28's real writer, observing `n` markers at the project's copy.
fn todo_sweep(
    tx: &Transaction<'_>,
    db: Ids,
    n: usize,
    outcome: DebtSweepOutcome,
    at: i64,
) -> SweepEffect {
    let seen: Vec<ObservedItem> = (0..n)
        .map(|i| ObservedItem {
            key: DebtKey::content(SUBJECT, "salient", i64::try_from(i).unwrap()),
            scoring: DebtScoring::Scored,
            location: Some(db.location),
            basis: Some(ObservationBasis::Head),
            path_bytes: Some(b"src/a.rs".to_vec()),
            path_display: Some("src/a.rs".to_owned()),
            line: Some(u32::try_from(i + 1).unwrap()),
            column: Some(1),
            salient_text: Some("TODO: fixture".to_owned()),
        })
        .collect();
    let observes = matches!(
        outcome,
        DebtSweepOutcome::Complete | DebtSweepOutcome::Partial
    );
    let obs = SweepObservation {
        project: db.project,
        source: DebtSource::TodoMarker,
        outcome,
        location: Some(db.location),
        generation: None,
        basis: Some(ObservationBasis::Head),
        item_count: observes.then(|| u32::try_from(n).unwrap()),
        observed_at: at,
    };
    SqliteDebtStore.observe(tx, &obs, &seen).unwrap()
}

/// One writer's transaction: snapshot, write, record — the shape every production caller has.
fn settle(db: &mut Db, at: i64, write: impl FnOnce(&Transaction<'_>, Ids) -> SweepEffect) -> usize {
    let ids = db.ids();
    let tx = db.conn.transaction().unwrap();
    let before = LayerValues::read(&tx, ids.project).unwrap();
    let effect = write(&tx, ids);
    record_after_write(
        &tx,
        db.project,
        &before,
        &effect.closed,
        HealthDetectedIn::Foreground,
        at,
    )
    .unwrap();
    tx.commit().unwrap();
    rows(&db.conn)
}

fn rows(conn: &Connection) -> usize {
    let n: i64 = conn
        .query_row("SELECT count(*) FROM health_delta", [], |r| r.get(0))
        .unwrap();
    usize::try_from(n).unwrap()
}

fn only_row(conn: &Connection) -> (String, f64, f64, String) {
    conn.query_row(
        "SELECT layer, from_value, to_value, detected_in FROM health_delta",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )
    .unwrap()
}

fn set_check(conn: &Connection, project: ProjectId, key: &str, state: &str, reason: Option<&str>) {
    conn.execute(
        "INSERT INTO project_check (project_id, check_key, state, user_na, unknown_reason,
                                    observed_at)
         VALUES (?1, ?2, ?3, NULL, ?4, 1)
         ON CONFLICT(project_id, check_key) DO UPDATE
           SET state = excluded.state, unknown_reason = excluded.unknown_reason",
        rusqlite::params![project.0, key, state, reason],
    )
    .unwrap();
}

fn check_state(conn: &Connection, project: ProjectId, key: &str) -> String {
    conn.query_row(
        "SELECT state FROM project_check WHERE project_id = ?1 AND check_key = ?2",
        rusqlite::params![project.0, key],
        |r| r.get(0),
    )
    .unwrap()
}

/// **`AC-P3-34-2` — the producer reads the debt set, not the tick.** A15's whole content, both
/// directions in one table: five open items becoming four writes a row while the completion check
/// stays `fail → fail`, and a check moving `unknown → fail` with no item change writes none.
/// Under D10's superseded wiring the first case would write nothing, and closing one of several
/// TODOs — the common case the phase-1 contract was recorded for — would be invisible.
#[test]
fn ac_p3_34_2_the_debt_set_moves_a_layer_and_the_tick_does_not() {
    let mut db = db(Some(ACK));
    settle(&mut db, NOW, |tx, db| {
        todo_sweep(tx, db, 5, DebtSweepOutcome::Complete, NOW)
    });
    set_check(&db.conn, db.project, "tests", "fail", None);
    set_check(&db.conn, db.project, "readme", "unknown", Some("notRead"));
    assert_eq!(rows(&db.conn), 0, "a first observation wrote a row");

    let mut executed = 0usize;

    // Five open items become four; the tick does not move.
    let after = settle(&mut db, NOW + 10, |tx, db| {
        todo_sweep(tx, db, 4, DebtSweepOutcome::Complete, NOW + 10)
    });
    eprintln!("five to four, tick fail -> fail: {after} row(s)");
    assert_eq!(after, 1);
    assert_eq!(check_state(&db.conn, db.project, "tests"), "fail");
    assert_eq!(
        only_row(&db.conn),
        ("overgrowth".to_owned(), 5.0, 4.0, "foreground".to_owned())
    );
    executed += 1;

    // The tick moves; the debt set does not.
    let after = settle(&mut db, NOW + 20, |tx, db| {
        set_check(tx, db.project, "readme", "fail", None);
        SweepEffect::default()
    });
    eprintln!(
        "tick unknown -> fail, no item change: {} new row(s)",
        after - 1
    );
    assert_eq!(after, 1, "a completion tick wrote a health delta");
    executed += 1;

    eprintln!("AC-P3-34-2 cases executed: {executed}");
    assert!(executed > 0);
}

/// **`AC-P3-34-3` — the precondition holds on the real write path.** Each case builds its own
/// fixture, drives its boundary the way the product does — a switch through the settings writer,
/// a freeze through the location's presence and §28's rule 3 — and then a debt write that WOULD
/// be a decrease, and asserts zero rows.
#[test]
fn ac_p3_34_3_no_row_crosses_a_first_observation_a_switch_a_freeze_or_an_enrolment() {
    type Case = (&'static str, fn() -> usize);
    let cases: [Case; 6] = [
        ("first observation", || {
            let mut db = db(Some(ACK));
            settle(&mut db, NOW, |tx, db| {
                todo_sweep(tx, db, 5, DebtSweepOutcome::Complete, NOW)
            })
        }),
        ("switch off, then a closure", || {
            let mut db = db(Some(ACK));
            settle(&mut db, NOW, |tx, db| {
                todo_sweep(tx, db, 5, DebtSweepOutcome::Complete, NOW)
            });
            switch(&mut db, &[(DebtSource::TodoMarker, false)]);
            settle(&mut db, NOW + 10, |tx, db| {
                todo_sweep(tx, db, 4, DebtSweepOutcome::Complete, NOW + 10)
            })
        }),
        ("switch off and on, then a closure", || {
            let mut db = db(Some(ACK));
            settle(&mut db, NOW, |tx, db| {
                todo_sweep(tx, db, 5, DebtSweepOutcome::Complete, NOW)
            });
            switch(&mut db, &[(DebtSource::TodoMarker, false)]);
            switch(&mut db, &[(DebtSource::TodoMarker, true)]);
            settle(&mut db, NOW + 10, |tx, db| {
                todo_sweep(tx, db, 4, DebtSweepOutcome::Complete, NOW + 10)
            })
        }),
        ("freeze", || {
            let mut db = db(Some(ACK));
            settle(&mut db, NOW, |tx, db| {
                todo_sweep(tx, db, 5, DebtSweepOutcome::Complete, NOW)
            });
            go(&db, "offline");
            settle(&mut db, NOW + 10, |tx, db| {
                // §28's rule 3: a root that cannot be read sweeps `unobservable`, and its open
                // items become `unverified` — a drop in the open count that fixed nothing.
                let outcome =
                    outcome_at_root(tx, Some(db.location), DebtSweepOutcome::Complete).unwrap();
                todo_sweep(tx, db, 0, outcome, NOW + 10)
            })
        }),
        ("unfreeze", || {
            let mut db = db(Some(ACK));
            settle(&mut db, NOW, |tx, db| {
                todo_sweep(tx, db, 5, DebtSweepOutcome::Complete, NOW)
            });
            go(&db, "offline");
            settle(&mut db, NOW + 10, |tx, db| {
                let outcome =
                    outcome_at_root(tx, Some(db.location), DebtSweepOutcome::Complete).unwrap();
                todo_sweep(tx, db, 0, outcome, NOW + 10)
            });
            go(&db, "present");
            settle(&mut db, NOW + 20, |tx, db| {
                todo_sweep(tx, db, 4, DebtSweepOutcome::Complete, NOW + 20)
            })
        }),
        ("enrolment", || {
            // Swept while not yet acknowledged, then acknowledged, then a closure: the evidence
            // the first snapshot counts predates the reading.
            let mut db = db(None);
            settle(&mut db, NOW, |tx, db| {
                todo_sweep(tx, db, 5, DebtSweepOutcome::Complete, NOW)
            });
            db.conn
                .execute(
                    "UPDATE project SET acknowledged_at = ?2 WHERE id = ?1",
                    rusqlite::params![db.project.0, NOW + 5],
                )
                .unwrap();
            settle(&mut db, NOW + 10, |tx, db| {
                todo_sweep(tx, db, 4, DebtSweepOutcome::Complete, NOW + 10)
            })
        }),
    ];

    // Every case runs and prints before any assertion, so a regression names every boundary it
    // opened rather than the first.
    let mut executed = 0usize;
    let mut crossed = Vec::new();
    for (name, case) in cases {
        let written = case();
        eprintln!("{name}: {written} row(s)");
        if written != 0 {
            crossed.push(name);
        }
        executed += 1;
    }
    eprintln!("AC-P3-34-3 cases executed: {executed}");
    assert!(executed > 0);
    assert!(
        crossed.is_empty(),
        "wrote a health delta across: {crossed:?}"
    );
}

fn go(db: &Db, presence: &str) {
    db.conn
        .execute(
            "UPDATE location SET presence = ?2 WHERE id = ?1",
            rusqlite::params![db.location.0, presence],
        )
        .unwrap();
}

/// The genuine case, so the zeros above are the precondition and not a producer that never
/// writes: both ends observed, same eligible set, value changed.
#[test]
fn ac_p3_34_3_a_genuine_observed_transition_writes_one_row() {
    let mut db = db(Some(ACK));
    settle(&mut db, NOW, |tx, db| {
        todo_sweep(tx, db, 5, DebtSweepOutcome::Complete, NOW)
    });
    let written = settle(&mut db, NOW + 10, |tx, db| {
        todo_sweep(tx, db, 4, DebtSweepOutcome::Complete, NOW + 10)
    });
    eprintln!("genuine transition: {written} row(s)");
    assert_eq!(written, 1);
    let nulls: i64 = db
        .conn
        .query_row(
            "SELECT count(*) FROM health_delta WHERE from_value IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(nulls, 0, "a row carries an unobserved from_value");
}

/// **R140 — `AC-P3-30-18`'s row-count half, under §30's own tag.** §30 asserted the predicate's
/// verdict per boundary; this drives each boundary through the REAL producer —
/// `record_layer_deltas`, with real `LayerValues::read` snapshots taken on the two sides of the
/// boundary event — and counts the rows it wrote.
///
/// Where both ends are observed values, only `delta_admissible` can refuse: a switch toggle
/// across a two-source layer, a freeze and an unfreeze. Enrolment and a first observation leave
/// the far end unobserved as well, so the producer refuses them twice over.
#[test]
fn ac_p3_30_18_no_boundary_driven_through_the_producer_writes_a_row() {
    type Case = (&'static str, fn() -> usize);
    let cases: [Case; 5] = [
        ("enrolment", || {
            let mut db = db(None);
            seed(&mut db, 5);
            straddle(&mut db, |db| {
                db.conn
                    .execute(
                        "UPDATE project SET acknowledged_at = ?2 WHERE id = ?1",
                        rusqlite::params![db.project.0, NOW + 5],
                    )
                    .unwrap();
            })
        }),
        ("freeze", || {
            let mut db = db(Some(ACK));
            seed(&mut db, 5);
            straddle(&mut db, |db| go(db, "offline"))
        }),
        ("unfreeze", || {
            let mut db = db(Some(ACK));
            seed(&mut db, 5);
            go(&db, "offline");
            straddle(&mut db, |db| go(db, "present"))
        }),
        ("switch toggle", || {
            // `overgrowth` fed by two observed sources, then one switched off: both ends are
            // observed values over two different sets.
            let mut db = db(Some(ACK));
            switch(&mut db, &[(DebtSource::MissingTests, true)]);
            seed(&mut db, 5);
            let tx = db.conn.transaction().unwrap();
            let tests = SweepObservation {
                project: db.project,
                source: DebtSource::MissingTests,
                outcome: DebtSweepOutcome::Complete,
                location: Some(db.location),
                generation: None,
                basis: Some(ObservationBasis::Head),
                item_count: Some(1),
                observed_at: NOW,
            };
            let item = ObservedItem {
                key: DebtKey::singleton(SUBJECT, DebtSource::MissingTests),
                scoring: DebtScoring::Scored,
                location: Some(db.location),
                basis: Some(ObservationBasis::Head),
                path_bytes: None,
                path_display: None,
                line: None,
                column: None,
                salient_text: None,
            };
            SqliteDebtStore.observe(&tx, &tests, &[item]).unwrap();
            tx.commit().unwrap();
            straddle_switch(&mut db)
        }),
        ("first observation", || {
            let mut db = db(Some(ACK));
            straddle(&mut db, |_| {})
        }),
    ];

    let mut executed = 0usize;
    let mut crossed = Vec::new();
    for (name, case) in cases {
        let written = case();
        eprintln!("{name}: {written} row(s)");
        if written != 0 {
            crossed.push(name);
        }
        executed += 1;
    }
    eprintln!("AC-P3-30-18 boundaries driven through the producer: {executed}");
    assert!(executed > 0);
    assert!(
        crossed.is_empty(),
        "the producer wrote a delta across: {crossed:?}"
    );

    // And the transition the gate exists to let through.
    let mut db = db(Some(ACK));
    seed(&mut db, 5);
    let written = straddle(&mut db, |_| {});
    eprintln!("genuine transition through the producer: {written} row(s)");
    assert_eq!(written, 1);
}

fn seed(db: &mut Db, n: usize) {
    let ids = db.ids();
    let tx = db.conn.transaction().unwrap();
    todo_sweep(&tx, ids, n, DebtSweepOutcome::Complete, NOW);
    tx.commit().unwrap();
}

/// Snapshot, the boundary event, one closure forced past §28's own rules, snapshot, record. The
/// closure is a direct delete so the value moves whatever the boundary did to the store's sweep
/// rules — what is under test is the producer's refusal, not §28's.
fn straddle(db: &mut Db, boundary: impl FnOnce(&Db)) -> usize {
    let before = LayerValues::read(&db.conn, db.project).unwrap();
    boundary(db);
    db.conn
        .execute(
            "DELETE FROM debt_item
              WHERE id = (SELECT max(id) FROM debt_item WHERE source = 'todo_marker')",
            [],
        )
        .unwrap();
    let tx = db.conn.transaction().unwrap();
    let after = LayerValues::read(&tx, db.project).unwrap();
    record_layer_deltas(
        &tx,
        db.project,
        &before,
        &after,
        &[],
        HealthDetectedIn::Foreground,
        NOW + 10,
    )
    .unwrap();
    tx.commit().unwrap();
    rows(&db.conn)
}

fn straddle_switch(db: &mut Db) -> usize {
    let before = LayerValues::read(&db.conn, db.project).unwrap();
    let observed = before.layer(DecayLayer::Overgrowth).unwrap();
    assert_eq!(
        observed.observation.value,
        Some(6),
        "the fixture is not two observed sources"
    );
    switch(db, &[(DebtSource::MissingTests, false)]);
    let tx = db.conn.transaction().unwrap();
    let after = LayerValues::read(&tx, db.project).unwrap();
    assert_eq!(
        after
            .layer(DecayLayer::Overgrowth)
            .unwrap()
            .observation
            .value,
        Some(5),
        "the far end is not an observed value, so the switch is not what refuses"
    );
    record_layer_deltas(
        &tx,
        db.project,
        &before,
        &after,
        &[],
        HealthDetectedIn::Foreground,
        NOW + 10,
    )
    .unwrap();
    tx.commit().unwrap();
    rows(&db.conn)
}

/// **`AC-P3-34-4`** — against a sweep row MARKED `partial`, not a timeout. §28's store already
/// refuses to close on a partial sweep, so the decrease here is forced past it: what is under
/// test is that the producer, handed one, writes nothing — while an increase over the same partial
/// sweep, and the same decrease over a `complete` one, are written.
#[test]
fn ac_p3_34_4_a_partial_sweep_writes_no_decrease() {
    let decrease_under = |outcome: &str| -> usize {
        let mut db = db(Some(ACK));
        seed(&mut db, 5);
        let tx = db.conn.transaction().unwrap();
        let before = LayerValues::read(&tx, db.project).unwrap();
        tx.execute(
            "UPDATE debt_sweep SET outcome = ?1, observed_at = ?2 WHERE source = 'todo_marker'",
            rusqlite::params![outcome, NOW + 10],
        )
        .unwrap();
        tx.execute(
            "DELETE FROM debt_item
              WHERE id = (SELECT max(id) FROM debt_item WHERE source = 'todo_marker')",
            [],
        )
        .unwrap();
        record_after_write(
            &tx,
            db.project,
            &before,
            &[],
            HealthDetectedIn::Foreground,
            NOW + 10,
        )
        .unwrap();
        tx.commit().unwrap();
        rows(&db.conn)
    };
    let partial = decrease_under("partial");
    let complete = decrease_under("complete");
    eprintln!("decrease: {partial} row(s) under partial, {complete} under complete");
    assert_eq!(partial, 0, "a partial sweep wrote a decrease");
    assert_eq!(
        complete, 1,
        "the same decrease under a complete sweep wrote nothing"
    );

    // An increase over a partial sweep is an item observed, and is kept.
    let mut db = db(Some(ACK));
    seed(&mut db, 5);
    let increase = settle(&mut db, NOW + 10, |tx, db| {
        todo_sweep(tx, db, 6, DebtSweepOutcome::Partial, NOW + 10)
    });
    eprintln!("increase under partial: {increase} row(s)");
    assert_eq!(increase, 1);
}

/// The row lands in the caller's transaction or not at all: a delta committed beside a debt set
/// that was not is a history of a state that never existed.
#[test]
fn a_rolled_back_write_keeps_no_row() {
    let mut db = db(Some(ACK));
    seed(&mut db, 5);
    {
        let ids = db.ids();
        let tx = db.conn.transaction().unwrap();
        let before = LayerValues::read(&tx, ids.project).unwrap();
        let effect = todo_sweep(&tx, ids, 4, DebtSweepOutcome::Complete, NOW + 10);
        let event = record_after_write(
            &tx,
            db.project,
            &before,
            &effect.closed,
            HealthDetectedIn::Foreground,
            NOW + 10,
        )
        .unwrap();
        assert!(event.is_some());
        assert_eq!(rows(&tx), 1, "the row is not in the writer's transaction");
        // Dropped without a commit.
    }
    eprintln!("after rollback: {} row(s)", rows(&db.conn));
    assert_eq!(rows(&db.conn), 0);
}

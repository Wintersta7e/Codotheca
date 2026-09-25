//! §28.4 — `debt_day`: one `xp_events` row per project per **local date**.
//!
//! **NEVER REWARD VOLUME** is an invariant, not a preference: deleting one file holding forty
//! markers is one keystroke, and a per-item key pays it 40×. The anti-volume rule is carried by a
//! UNIQUE constraint, not by a cap bolted on afterwards.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::debt::identity::DebtKey;
use codotheca_core::debt::store::{DebtCloseReason, DebtClosure, SweepEffect};
use codotheca_core::debt::xp::{debt_day_dedupe_key, pay_debt_day};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::subject::ProjectSubject;
use codotheca_core::index::{open_connection, Index};
use codotheca_core::protocol::{DebtScoring, DebtSource, ProjectId};

/// 2026-09-17 12:00:00 UTC — mid-day, so a small offset does not cross a boundary by accident.
const NOON: i64 = 1_789_646_400;
const DAY: i64 = 86_400;

fn fresh() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

fn insert_project(conn: &rusqlite::Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES (?1, ?1, 1, 1)",
        [name],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn lineage_subject() -> ProjectSubject {
    ProjectSubject::Lineage {
        lineage_key: "abc123".to_owned(),
        remote_key: Some("github.com/o/r".to_owned()),
    }
}

/// A project with **no lineage** keys on its path, which §1.7's git shape cannot express.
fn path_subject(byte: u8) -> ProjectSubject {
    ProjectSubject::Path {
        kind: "linux".to_owned(),
        distro: String::new(),
        path_key: vec![byte],
    }
}

/// The `i`th closure of `source`, distinct from every other `i`.
fn closure(
    i: usize,
    reason: DebtCloseReason,
    scoring: DebtScoring,
    source: DebtSource,
) -> DebtClosure {
    let mut key = DebtKey::content("subject", &format!("{i:064x}"), 0);
    key.source = source;
    DebtClosure {
        key,
        reason,
        scoring,
    }
}

fn closures(n: usize, reason: DebtCloseReason, source: DebtSource) -> SweepEffect {
    SweepEffect {
        closed: (0..n)
            .map(|i| closure(i, reason, DebtScoring::Scored, source))
            .collect(),
        ..SweepEffect::default()
    }
}

fn xp_count(conn: &rusqlite::Connection) -> i64 {
    conn.query_row("SELECT count(*) FROM xp_events", [], |r| r.get(0))
        .unwrap()
}

fn row(conn: &rusqlite::Connection) -> (i64, Option<i64>, String, String, String, String) {
    conn.query_row(
        "SELECT ts, tz_offset_min, kind, track, subject_key, dedupe_key FROM xp_events",
        [],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        },
    )
    .unwrap()
}

fn meta(conn: &rusqlite::Connection) -> serde_json::Value {
    let raw: String = conn
        .query_row("SELECT meta FROM xp_events", [], |r| r.get(0))
        .unwrap();
    serde_json::from_str(&raw).unwrap()
}

/// **`AC-P3-28-5`.** Forty items closed in one local date write **exactly one** row. The second
/// closure updates `meta` and **no other column** — the payout is the row's existence and that
/// never changes, while `meta` is the day's sentence and a sentence describing only the first
/// closure undercounts the day it claims to describe.
#[test]
fn ac_p3_28_5_forty_closures_in_one_day_pay_once() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let subject = lineage_subject().to_key();

    let tx = conn.transaction().unwrap();
    let first = pay_debt_day(
        &tx,
        ProjectId(p),
        &subject,
        &closures(40, DebtCloseReason::Fixed, DebtSource::TodoMarker),
        NOON,
        0,
    )
    .unwrap();
    tx.commit().unwrap();

    assert!(first.wrote_row);
    assert_eq!(first.closed_today, 40);
    assert_eq!(first.sources, vec![DebtSource::TodoMarker]);

    let rows: i64 = conn
        .query_row("SELECT count(*) FROM xp_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1, "forty items paid more than once");
    assert_eq!(meta(&conn)["closed"], 40);
    assert_eq!(meta(&conn)["sources"], serde_json::json!(["todo_marker"]));

    let before = row(&conn);
    assert_eq!(before.2, "debt_day");
    assert_eq!(before.3, "session");
    assert_eq!(before.4, subject);
    assert_eq!(
        before.5,
        debt_day_dedupe_key(&subject, "2026-09-17"),
        "the dedupe key is built in one place and this is it"
    );

    // A second closure, later the same local day, from a different source.
    let second_tx = conn.transaction().unwrap();
    let second = pay_debt_day(
        &second_tx,
        ProjectId(p),
        &subject,
        &closures(2, DebtCloseReason::Fixed, DebtSource::MissingReadme),
        NOON + 3_600,
        0,
    )
    .unwrap();
    second_tx.commit().unwrap();

    assert!(!second.wrote_row, "the second closure of a day paid again");
    assert_eq!(second.closed_today, 42, "meta must describe the whole day");

    let rows_after_second: i64 = conn
        .query_row("SELECT count(*) FROM xp_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows_after_second, 1);
    assert_eq!(meta(&conn)["closed"], 42);
    assert_eq!(
        meta(&conn)["sources"],
        serde_json::json!(["missing_readme", "todo_marker"]),
    );

    assert_eq!(
        row(&conn),
        before,
        "the second closure moved a column that is never updated"
    );
}

/// **`AC-P3-28-15`.** `Invalidated` is a third party retracting the evidence, not the user
/// acting, and it **pays nothing**. A day whose only closures are invalidated writes **no row at
/// all** — and an XP row already written for that project is untouched, because nothing earned is
/// ever removed.
#[test]
fn ac_p3_28_15_an_invalidated_closure_pays_nothing() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let subject = lineage_subject().to_key();

    let tx = conn.transaction().unwrap();
    let only_withdrawn = pay_debt_day(
        &tx,
        ProjectId(p),
        &subject,
        &closures(
            3,
            DebtCloseReason::Invalidated,
            DebtSource::DependencyAdvisory,
        ),
        NOON,
        0,
    )
    .unwrap();
    tx.commit().unwrap();

    assert!(!only_withdrawn.wrote_row);
    assert_eq!(only_withdrawn.closed_today, 0);
    assert!(only_withdrawn.sources.is_empty());
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM xp_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 0, "a withdrawn advisory paid");

    // A real closure the same day pays, and a withdrawal afterwards leaves it alone.
    let fixed_tx = conn.transaction().unwrap();
    pay_debt_day(
        &fixed_tx,
        ProjectId(p),
        &subject,
        &closures(1, DebtCloseReason::Fixed, DebtSource::TodoMarker),
        NOON,
        0,
    )
    .unwrap();
    fixed_tx.commit().unwrap();
    let earned = row(&conn);
    assert_eq!(meta(&conn)["closed"], 1);

    let withdrawn_tx = conn.transaction().unwrap();
    pay_debt_day(
        &withdrawn_tx,
        ProjectId(p),
        &subject,
        &closures(
            5,
            DebtCloseReason::Invalidated,
            DebtSource::DependencyAdvisory,
        ),
        NOON + 60,
        0,
    )
    .unwrap();
    withdrawn_tx.commit().unwrap();

    assert_eq!(row(&conn), earned, "a withdrawal rewrote an earned row");
    assert_eq!(meta(&conn)["closed"], 1, "a withdrawal entered the count");
    assert_eq!(meta(&conn)["sources"], serde_json::json!(["todo_marker"]));
}

/// **§38.8.1 gate 1, §28.6.** Only a closure of a `scored` item pays. A day whose only closure
/// is `shown_only` writes no row, and on a day that mixes the two the `shown_only` closure enters
/// neither `closed` nor `sources` — the treatment `Invalidated` already gets.
#[test]
fn a_shown_only_closure_pays_nothing_and_leaves_the_day_to_what_pays() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let subject = lineage_subject().to_key();

    let tx = conn.transaction().unwrap();
    let alone = pay_debt_day(
        &tx,
        ProjectId(p),
        &subject,
        &SweepEffect {
            closed: vec![closure(
                0,
                DebtCloseReason::Fixed,
                DebtScoring::ShownOnly,
                DebtSource::DependencyAdvisory,
            )],
            ..SweepEffect::default()
        },
        NOON,
        0,
    )
    .unwrap();
    tx.commit().unwrap();
    assert!(!alone.wrote_row, "a shown_only closure paid");
    assert_eq!(xp_count(&conn), 0);

    let mixed_tx = conn.transaction().unwrap();
    let mixed = pay_debt_day(
        &mixed_tx,
        ProjectId(p),
        &subject,
        &SweepEffect {
            closed: vec![
                closure(
                    1,
                    DebtCloseReason::Fixed,
                    DebtScoring::Scored,
                    DebtSource::TodoMarker,
                ),
                closure(
                    2,
                    DebtCloseReason::Fixed,
                    DebtScoring::ShownOnly,
                    DebtSource::DependencyAdvisory,
                ),
            ],
            ..SweepEffect::default()
        },
        NOON + 60,
        0,
    )
    .unwrap();
    mixed_tx.commit().unwrap();

    assert!(mixed.wrote_row);
    assert_eq!(xp_count(&conn), 1);
    assert_eq!(
        meta(&conn)["closed"],
        1,
        "a shown_only closure entered the count"
    );
    assert_eq!(
        meta(&conn)["sources"],
        serde_json::json!(["todo_marker"]),
        "a shown_only closure named its source"
    );
}

/// **The key is `<subject_key>` and not §1.7's `<lineage_key>:<remote_key>`.** That shape is safe
/// for `commit_day` only because a project with no lineage has no commits; a project with no
/// lineage can absolutely have markers, and every such project would collapse onto
/// `debt_day:::<date>`, where `dedupe_key`'s UNIQUE plus `ON CONFLICT DO NOTHING` turns the
/// collision into **silent non-payment** rather than an error.
#[test]
fn two_projects_with_no_lineage_do_not_collide_on_one_date() {
    let (_d, mut conn) = fresh();
    let a = insert_project(&conn, "a");
    let b = insert_project(&conn, "b");

    let tx = conn.transaction().unwrap();
    for (project, byte) in [(a, 0xaa_u8), (b, 0xbb_u8)] {
        let paid = pay_debt_day(
            &tx,
            ProjectId(project),
            &path_subject(byte).to_key(),
            &closures(1, DebtCloseReason::Fixed, DebtSource::TodoMarker),
            NOON,
            0,
        )
        .unwrap();
        assert!(paid.wrote_row, "project {project} was not paid");
    }
    tx.commit().unwrap();

    let rows: i64 = conn
        .query_row("SELECT count(*) FROM xp_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        rows, 2,
        "two unlineaged projects collapsed onto one dedupe key"
    );
}

/// Two closures either side of the **local** midnight are two days and two rows. The offset is
/// the one the local date was computed in, so a row is readable in the frame it was earned in.
#[test]
fn closures_either_side_of_local_midnight_are_two_days() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    let subject = lineage_subject().to_key();

    let tx = conn.transaction().unwrap();
    pay_debt_day(
        &tx,
        ProjectId(p),
        &subject,
        &closures(1, DebtCloseReason::Fixed, DebtSource::TodoMarker),
        NOON,
        0,
    )
    .unwrap();
    pay_debt_day(
        &tx,
        ProjectId(p),
        &subject,
        &closures(1, DebtCloseReason::Fixed, DebtSource::TodoMarker),
        NOON + DAY,
        0,
    )
    .unwrap();
    tx.commit().unwrap();

    let rows: i64 = conn
        .query_row("SELECT count(*) FROM xp_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 2);

    // The same two moments in a frame that moves the boundary: 13:00 and 13:00 the next day in
    // UTC+13 are still two local days, and the stored offset says which frame.
    let (_d2, mut other) = fresh();
    let q = insert_project(&other, "thing");
    let other_tx = other.transaction().unwrap();
    pay_debt_day(
        &other_tx,
        ProjectId(q),
        &subject,
        &closures(1, DebtCloseReason::Fixed, DebtSource::TodoMarker),
        NOON,
        780,
    )
    .unwrap();
    other_tx.commit().unwrap();
    let stored_offset: Option<i64> = other
        .query_row("SELECT tz_offset_min FROM xp_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        stored_offset,
        Some(780),
        "the offset the local date was computed in is not a synthetic zero"
    );
}

/// A sweep that closed nothing writes nothing. There is no zero-payout row.
#[test]
fn a_sweep_that_closed_nothing_writes_no_row() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");

    let tx = conn.transaction().unwrap();
    let paid = pay_debt_day(
        &tx,
        ProjectId(p),
        &lineage_subject().to_key(),
        &SweepEffect::default(),
        NOON,
        0,
    )
    .unwrap();
    tx.commit().unwrap();

    assert!(!paid.wrote_row);
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM xp_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 0);
}

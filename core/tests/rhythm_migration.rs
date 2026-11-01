//! `0020_rhythm.sql` — §39.12's `rhythm_week`, raise-only in the schema, and the three push-record
//! columns J1 fills; plus the week's trip through the sidecar (§48.8.3's raise-only restore).

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;

use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::rebuild::{rebuild_in_place, RebuildOutcome};
use codotheca_core::index::sidecar::{counts, read, RestoreCtx, RestoreOutcome, SECTIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::protocol::RhythmCoverage;
use codotheca_core::rings::rhythm::{load_weeks, restore_week, RhythmVerdict, RhythmWeekRow};
use codotheca_core::testing::sidecar::{FixtureIds, SECTION_FIXTURES};
use rusqlite::Connection;

const NOW: i64 = 1_760_000_000;

/// A database with every shipped migration applied, and the directory that holds it.
fn fresh() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

/// The column's spelling of a wire coverage: the schema's own, read through serde.
fn coverage_slug(coverage: RhythmCoverage) -> String {
    serde_json::to_value(coverage)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}

fn insert(conn: &Connection, week: &str, days: i64, target: i64, verdict: &str, coverage: &str) {
    conn.execute(
        "INSERT INTO rhythm_week
            (week_start, active_days, target, verdict, git_coverage, settled_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 1)",
        rusqlite::params![week, days, target, verdict, coverage],
    )
    .unwrap();
}

#[test]
fn every_rhythm_coverage_variant_is_accepted_by_the_column() {
    let (_dir, conn) = fresh();
    let mut inserted = 0;
    for (i, coverage) in RhythmCoverage::ALL.into_iter().enumerate() {
        let week = format!("2026-09-{:02}", 7 + 7 * i);
        insert(&conn, &week, 0, 3, "missed", &coverage_slug(coverage));
        inserted += 1;
    }
    eprintln!("inserted {inserted} of {}", RhythmCoverage::ALL.len());
    assert_ne!(inserted, 0, "no coverage variant was inserted");
    assert_eq!(inserted, RhythmCoverage::ALL.len());
}

#[test]
fn every_verdict_slug_is_accepted_by_the_column() {
    let (_dir, conn) = fresh();
    let mut inserted = 0;
    for (i, verdict) in RhythmVerdict::ALL.into_iter().enumerate() {
        let week = format!("2026-09-{:02}", 7 + 7 * i);
        // The verdict CHECK: `counted` exactly when the days reach the target.
        let days = if verdict == RhythmVerdict::Counted {
            3
        } else {
            1
        };
        insert(&conn, &week, days, 3, verdict.as_str(), "complete");
        inserted += 1;
    }
    eprintln!("inserted {inserted} of {}", RhythmVerdict::ALL.len());
    assert_ne!(inserted, 0, "no verdict was inserted");
    assert_eq!(inserted, RhythmVerdict::ALL.len());
}

/// **A settled week only rises.** Every lowering, rewrite, deletion and second insert aborts; the
/// one raise commits.
#[test]
fn ac_p4_39_8_the_triggers_refuse_every_lowering() {
    let (_dir, conn) = fresh();
    insert(&conn, "2026-09-14", 2, 3, "missed", "partial");
    let week = "WHERE week_start = '2026-09-14'";
    let cases: Vec<(&str, String, bool)> = vec![
        (
            "a lower active_days",
            format!("UPDATE rhythm_week SET active_days = 1 {week}"),
            false,
        ),
        (
            "a target change",
            format!("UPDATE rhythm_week SET target = 2 {week}"),
            false,
        ),
        (
            "a settled_at change",
            format!("UPDATE rhythm_week SET settled_at = 2 {week}"),
            false,
        ),
        (
            "a week_start change",
            format!("UPDATE rhythm_week SET week_start = '2026-09-21' {week}"),
            false,
        ),
        ("a DELETE", format!("DELETE FROM rhythm_week {week}"), false),
        (
            "INSERT OR REPLACE of the week",
            "INSERT OR REPLACE INTO rhythm_week VALUES ('2026-09-14', 2, 3, 'missed', 'partial', 1)"
                .to_owned(),
            false,
        ),
        (
            "an upsert on the week",
            "INSERT INTO rhythm_week VALUES ('2026-09-14', 2, 3, 'missed', 'partial', 1)
             ON CONFLICT (week_start) DO UPDATE SET active_days = excluded.active_days"
                .to_owned(),
            false,
        ),
        (
            "INSERT OR IGNORE of the week",
            "INSERT OR IGNORE INTO rhythm_week VALUES ('2026-09-14', 2, 3, 'missed', 'partial', 1)"
                .to_owned(),
            false,
        ),
        (
            "an insert violating the verdict CHECK",
            "INSERT INTO rhythm_week VALUES ('2026-09-28', 1, 3, 'counted', 'complete', 1)"
                .to_owned(),
            false,
        ),
        (
            "the raise to counted and complete",
            format!(
                "UPDATE rhythm_week SET active_days = 3, verdict = 'counted',
                 git_coverage = 'complete' {week}"
            ),
            true,
        ),
        (
            "counted back to missed",
            format!("UPDATE rhythm_week SET active_days = 3, verdict = 'missed' {week}"),
            false,
        ),
        (
            "complete back to partial",
            format!("UPDATE rhythm_week SET git_coverage = 'partial' {week}"),
            false,
        ),
    ];
    let mut mismatches = Vec::new();
    for (name, sql, should_commit) in &cases {
        let committed = conn.execute_batch(sql).is_ok();
        eprintln!("{name}: {}", if committed { "accepted" } else { "refused" });
        if committed != *should_commit {
            mismatches.push(*name);
        }
    }
    assert_ne!(cases.len(), 0);
    assert_eq!(mismatches, Vec::<&str>::new(), "the triggers disagree");
    let row: (i64, String, String) = conn
        .query_row(
            &format!("SELECT active_days, verdict, git_coverage FROM rhythm_week {week}"),
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(row, (3, "counted".to_owned(), "complete".to_owned()));
}

#[test]
fn the_push_columns_start_unobserved() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    let before = MIGRATIONS.iter().position(|m| m.version == 20).unwrap();
    apply_all(&mut conn, &MIGRATIONS[..before]).unwrap();
    conn.execute_batch(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
         VALUES ('held', 'held', 'held-lineage', 1, 1);
         INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind)
         VALUES (last_insert_rowid(), 'linux', X'2F68656C64', X'2F68656C64', '/held', 'fixture',
                 'present', 'worktree');",
    )
    .unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    let row: (Option<i64>, Option<i64>, Option<String>) = conn
        .query_row(
            "SELECT last_push_at, last_push_tz_offset_min, push_record FROM location",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        (None, None, None),
        "a migrated copy reads not computed"
    );
}

/// Corrupt the index in `dir` and rebuild it through the production entry.
fn corrupt_and_rebuild(dir: &Path, now: i64) -> Index {
    std::fs::write(Index::db_path(dir), b"this is not a database").unwrap();
    match rebuild_in_place(dir, now) {
        Ok(RebuildOutcome::Rebuilt(report)) => {
            eprintln!("rebuilt: restored {:?}", report.restored);
        }
        other => panic!("expected a rebuild, got {other:?}"),
    }
    Index::open_at(dir, now + 1).unwrap()
}

fn restore(index: &mut Index, row: &codotheca_core::index::sidecar::SectionRow) -> RestoreOutcome {
    let spec = SECTIONS.iter().find(|s| s.name == "rhythm_week").unwrap();
    index
        .with_tx(|tx| {
            let ctx = RestoreCtx {
                now: NOW,
                project: None,
                locations: &[],
                source_generation: 1,
            };
            (spec.restore)(tx, row, &ctx)
        })
        .unwrap()
}

#[test]
fn a_settled_week_round_trips_through_the_sidecar() {
    let fixture = SECTION_FIXTURES
        .iter()
        .find(|(name, _)| *name == "rhythm_week")
        .map(|(_, f)| *f)
        .expect("rhythm_week has a fixture");
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open_at(dir.path(), NOW).unwrap();
    index
        .with_tx(|tx| fixture(tx, &FixtureIds::default()))
        .unwrap();
    let before = load_weeks(index.conn()).unwrap();
    assert_eq!(before.len(), 2, "the fixture settles two weeks");
    let doc = read(&index.export_sidecar(NOW + 10).unwrap().path).unwrap();
    assert_eq!(counts(&doc).get("rhythm_week").copied(), Some(2));
    drop(index);

    let rebuilt = corrupt_and_rebuild(dir.path(), NOW + 20);
    let after = load_weeks(rebuilt.conn()).unwrap();
    eprintln!("rows restored: {}", after.len());
    assert_eq!(after, before, "the settled weeks come back as they were");
    drop(rebuilt);

    // Over a lower present row the restore raises it; over a higher one it changes nothing.
    let rows = &doc.payload.sections["rhythm_week"];
    let fresh_dir = tempfile::tempdir().unwrap();
    let mut held = Index::open_at(fresh_dir.path(), NOW).unwrap();
    let lower = &before[1];
    held.conn()
        .execute(
            "INSERT INTO rhythm_week VALUES (?1, ?2, ?3, 'missed', 'partial', ?4)",
            rusqlite::params![
                lower.week_start,
                lower.target - 1,
                lower.target,
                lower.settled_at
            ],
        )
        .unwrap();
    let raised = rows
        .iter()
        .find(|r| r.data["week_start"] == lower.week_start)
        .unwrap();
    assert_eq!(restore(&mut held, raised), RestoreOutcome::Applied(1));
    let now_held: Vec<RhythmWeekRow> = load_weeks(held.conn()).unwrap();
    assert_eq!(now_held, vec![lower.clone()], "the lower row was raised");

    held.conn()
        .execute(
            "UPDATE rhythm_week SET active_days = 7 WHERE week_start = ?1",
            [&lower.week_start],
        )
        .unwrap();
    let higher = load_weeks(held.conn()).unwrap();
    assert_eq!(restore(&mut held, raised), RestoreOutcome::Applied(0));
    assert_eq!(
        load_weeks(held.conn()).unwrap(),
        higher,
        "a restore lowered a raised week"
    );
    assert_eq!(higher[0].active_days, 7);
    assert_eq!(higher[0].verdict, RhythmVerdict::Counted);
}

/// The writer enforces what the column cannot: a settled week starts on a Monday, written
/// `YYYY-MM-DD`.
#[test]
fn a_week_that_is_not_a_monday_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open_at(dir.path(), NOW).unwrap();
    let week = |week_start: &str| RhythmWeekRow {
        week_start: week_start.to_owned(),
        active_days: 1,
        target: 3,
        verdict: RhythmVerdict::Missed,
        git_coverage: RhythmCoverage::Complete,
        settled_at: NOW,
    };
    let mut refused = 0;
    for bad in ["2026-09-15", "2026-09-20", "2026-9-14", "2026-02-30", ""] {
        let result = index.with_tx(|tx| restore_week(tx, &week(bad)));
        assert!(result.is_err(), "{bad:?} was written");
        refused += 1;
    }
    eprintln!("refused {refused}");
    assert_eq!(load_weeks(index.conn()).unwrap(), Vec::new());
    assert!(index
        .with_tx(|tx| restore_week(tx, &week("2026-09-14")))
        .unwrap());
    assert_eq!(load_weeks(index.conn()).unwrap().len(), 1);
}

//! §38.10: the ledger's retained half through a corrupt-index rebuild — each per-project row when
//! its subject returns, the library's own award, the floor as a raise, and the git rows of a
//! project no scan can find again — and a level read before the corruption is not lower after it.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;

use codotheca_core::clock::LocalInstant;
use codotheca_core::index::pending::match_pending;
use codotheca_core::index::rebuild::{rebuild_in_place, RebuildOutcome};
use codotheca_core::index::sidecar::{
    counts, read, unclassified, RestoreCtx, RestoreOutcome, Sidecar, SECTIONS,
};
use codotheca_core::index::subject::subject_for_project;
use codotheca_core::index::Index;
use codotheca_core::ledger::floor::{ratchet_level_floor, read_floor};
use codotheca_core::ledger::focus::credit_segment_close;
use codotheca_core::ledger::git_rows::{reconcile_git_rows, GitRow};
use codotheca_core::ledger::level::read_level;
use codotheca_core::ledger::verdict::pay_prepass_award;
use codotheca_core::ledger::{LEGACY_FLOOR_KEY, RETAINED_KINDS};
use codotheca_core::protocol::{LevelState, ProjectId};
use codotheca_core::testing::zone::FixedZone;
use rusqlite::{Connection, OptionalExtension as _};

const NOW: i64 = 1_760_000_000;

/// The kept project's lineage and remote: a current project, so its rows wait for its subject.
const KEPT: (&str, &str) = ("kept-lineage", "kept-remote");
/// The frozen project's: every copy removed, so only the sidecar can bring it back.
const FROZEN: (&str, &str) = ("frozen-lineage", "frozen-remote");

/// One `xp_events` row as a restore must bring it back: kind, key, time and receipt.
type XpRow = (String, String, i64, Option<String>);

/// The rows `filter` selects, by key.
fn xp_rows(conn: &Connection, filter: &str) -> Vec<XpRow> {
    conn.prepare(&format!(
        "SELECT kind, dedupe_key, ts, meta FROM xp_events WHERE {filter} ORDER BY dedupe_key"
    ))
    .unwrap()
    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

/// The `project_id` of every git row, by key.
fn git_owners(conn: &Connection) -> Vec<i64> {
    conn.prepare("SELECT project_id FROM xp_events WHERE track = 'git' ORDER BY dedupe_key")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// The `signalled` of each segment of the session that started at 100.
fn signalled(conn: &Connection) -> Vec<Option<i64>> {
    conn.prepare(
        "SELECT g.signalled FROM session_segment g JOIN session s ON s.id = g.session_id
          WHERE s.started_at = 100 ORDER BY g.started_at",
    )
    .unwrap()
    .query_map([], |r| r.get(0))
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

/// A project on `lineage` with one copy at `path`.
fn project(
    conn: &Connection,
    (lineage, remote): (&str, &str),
    path: &str,
    presence: &str,
    removed_at: Option<i64>,
) -> ProjectId {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, remote_key, created_at, updated_at)
         VALUES (?1, ?1, ?1, ?2, 1, 1)",
        rusqlite::params![lineage, remote],
    )
    .unwrap();
    let id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind, removed_at)
         VALUES (?1, 'linux', ?2, ?2, ?3, 's', ?4, 'worktree', ?5)",
        rusqlite::params![id, path.as_bytes(), path, presence, removed_at],
    )
    .unwrap();
    ProjectId(id)
}

/// The library before the corruption: the kept project with one row of every retained kind and a
/// session whose segments were signalled, not signalled and never asked; the frozen project with
/// two git rows; the library's pre-pass award; and the identity card answered. The kept copy is
/// missing, so no project is pending and the reading is computed.
fn seed(index: &mut Index) -> ProjectId {
    index
        .with_tx(|tx| {
            let kept = project(tx, KEPT, "/fixture/kept", "missing", None);
            let subject = subject_for_project(tx, kept)?.unwrap().to_key();
            for (ts, kind) in (NOW - 100..).zip(RETAINED_KINDS) {
                let meta = if kind == "focus" {
                    r#"{"seconds":1800}"#
                } else {
                    "{}"
                };
                tx.execute(
                    "INSERT INTO xp_events
                        (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key, track, meta)
                     VALUES (?1, 0, ?2, ?3, ?4, ?5, 'session', ?6)",
                    rusqlite::params![ts, kept.0, subject, kind, format!("{kind}:{subject}"), meta],
                )?;
            }
            tx.execute(
                "INSERT INTO session (project_id, started_at, ended_at, credited_seconds,
                                      close_reason)
                 VALUES (?1, 100, 400, 300, 'stop')",
                [kept.0],
            )?;
            let session = tx.last_insert_rowid();
            for (start, signalled) in [(100, Some(1)), (200, Some(0)), (300, None::<i64>)] {
                tx.execute(
                    "INSERT INTO session_segment
                        (session_id, started_at, ended_at, credited_seconds, closed_by, signalled)
                     VALUES (?1, ?2, ?3, 100, 'idle', ?4)",
                    rusqlite::params![session, start, start + 100, signalled],
                )?;
            }

            let frozen = project(tx, FROZEN, "/fixture/frozen", "missing", Some(50));
            let frozen_subject = subject_for_project(tx, frozen)?.unwrap().to_key();
            let rows = [19_000_i64, 19_001].map(|day| GitRow {
                kind: "commit_day",
                dedupe_key: format!("commit_day:{frozen_subject}:{day}"),
                subject_key: frozen_subject.clone(),
                ts: day * 86_400,
                meta: "{}".to_owned(),
            });
            reconcile_git_rows(tx, frozen, &rows, NOW)?;

            let at = LocalInstant {
                unix: NOW - 50,
                offset_min: 0,
            };
            pay_prepass_award(tx, NOW - 50, "kept group", 2, at)?;
            tx.execute(
                "INSERT INTO view_state (k, v) VALUES ('notice.dismissed.identity', '1')",
                [],
            )?;
            Ok(frozen)
        })
        .unwrap()
}

/// What the library held before the corruption, as a restore must bring it back.
struct Before {
    retained: Vec<XpRow>,
    award: Vec<XpRow>,
    frozen: Vec<XpRow>,
    level: Option<u32>,
}

/// The computed, answered read that ratchets the floor, then the export.
fn read_and_export(index: &mut Index) -> (Before, Sidecar) {
    let reading = index.with_tx(|tx| read_level(tx, NOW)).unwrap();
    assert_eq!(reading.state, LevelState::Computed);
    assert_eq!(reading.level, Some(2));
    assert_eq!(
        read_floor(index.conn()).unwrap(),
        2,
        "the read raised the floor"
    );
    let conn = index.conn();
    let before = Before {
        retained: xp_rows(conn, "track = 'session' AND project_id IS NOT NULL"),
        award: xp_rows(conn, "project_id IS NULL"),
        frozen: xp_rows(conn, "track = 'git'"),
        level: reading.level,
    };
    assert_eq!(before.retained.len(), RETAINED_KINDS.len());
    assert_eq!(signalled(conn), [Some(1), Some(0), None]);
    let doc = read(&index.export_sidecar(NOW + 10).unwrap().path).unwrap();
    let carried = counts(&doc);
    for (section, n) in [
        ("ledger_floor", 1),
        ("ledger_library_awards", 1),
        ("ledger_frozen_git", 2),
    ] {
        eprintln!(
            "{section}: {} exported",
            carried.get(section).copied().unwrap_or(0)
        );
        assert_eq!(carried.get(section).copied(), Some(n), "{section}");
    }
    (before, doc)
}

/// Corrupt the index in `dir` and rebuild it through the production entry.
fn corrupt_and_rebuild(dir: &Path, now: i64) -> Index {
    std::fs::write(Index::db_path(dir), b"this is not a database").unwrap();
    match rebuild_in_place(dir, now) {
        Ok(RebuildOutcome::Rebuilt(report)) => eprintln!(
            "rebuilt: restored {:?}, {} pending",
            report.restored, report.pending
        ),
        other => panic!("expected a rebuild, got {other:?}"),
    }
    Index::open_at(dir, now + 1).unwrap()
}

/// The kept project comes back as a scan would bring it: no retained writer writes for it while
/// its records wait, and the match brings every retained row and each segment's `signalled` back.
fn the_kept_project_returns(index: &mut Index, before: &Before) {
    assert_eq!(
        xp_rows(index.conn(), "track = 'session' AND project_id IS NOT NULL"),
        [],
        "nothing of the kept project's restores before its subject returns"
    );
    index
        .with_tx(|tx| {
            let back = project(tx, KEPT, "/fixture/kept-back", "present", None);
            tx.execute(
                "INSERT INTO session (project_id, started_at, ended_at, credited_seconds,
                                      close_reason)
                 VALUES (?1, ?2, ?3, 600, 'stop')",
                rusqlite::params![back.0, NOW + 100, NOW + 700],
            )?;
            tx.execute(
                "INSERT INTO session_segment
                    (session_id, started_at, ended_at, credited_seconds, closed_by, signalled)
                 VALUES (?1, ?2, ?3, 600, 'session_end', 1)",
                rusqlite::params![tx.last_insert_rowid(), NOW + 100, NOW + 700],
            )?;
            let segment = tx.last_insert_rowid();
            let held = credit_segment_close(tx, &FixedZone(0), segment).unwrap();
            assert_eq!(
                held, 0,
                "a retained writer wrote before the records matched"
            );

            let matched = match_pending(tx, back, NOW + 800)?;
            eprintln!("matched: {:?}", matched.applied);
            Ok(())
        })
        .unwrap();
    let conn = index.conn();
    assert_eq!(
        xp_rows(conn, "track = 'session' AND project_id IS NOT NULL"),
        before.retained
    );
    assert_eq!(signalled(conn), [Some(1), Some(0), None]);
}

/// A lower floor restored through the registered restore over a higher one changes nothing and
/// aborts nothing.
fn a_lower_floor_changes_nothing(index: &mut Index, doc: &Sidecar) {
    let spec = SECTIONS
        .iter()
        .find(|s| s.name == "ledger_floor")
        .expect("the floor's section is registered");
    let row = &doc.payload.sections["ledger_floor"][0];
    let outcome = index
        .with_tx(|tx| {
            ratchet_level_floor(tx, 5, NOW + 900)?;
            let ctx = RestoreCtx {
                now: NOW + 900,
                project: None,
                locations: &[],
                source_generation: doc.generation,
            };
            (spec.restore)(tx, row, &ctx)
        })
        .unwrap();
    assert_eq!(outcome, RestoreOutcome::Applied(0));
    assert_eq!(read_floor(index.conn()).unwrap(), 5);
}

/// A format-1 sidecar carrying the legacy key restores no floor and no `app_meta` row for it.
fn the_legacy_key_restores_no_floor() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sidecar-format1.json");
    let doc = read(&fixture).unwrap();
    assert_eq!(doc.format, 1);
    assert!(
        doc.payload.settings.contains_key(LEGACY_FLOOR_KEY),
        "the fixture must carry the legacy key for its drop to mean anything"
    );
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(&fixture, Index::sidecar_path(dir.path())).unwrap();
    let index = corrupt_and_rebuild(dir.path(), NOW);
    let legacy: Option<String> = index
        .conn()
        .query_row(
            "SELECT v FROM app_meta WHERE k = ?1",
            [LEGACY_FLOOR_KEY],
            |r| r.get(0),
        )
        .optional()
        .unwrap();
    assert_eq!(legacy, None);
    assert_eq!(read_floor(index.conn()).unwrap(), 1);
}

#[test]
fn ac_p4_38_23_the_sidecar_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let mut original = Index::open_at(dir.path(), NOW).unwrap();
    let frozen = seed(&mut original);
    let (before, doc) = read_and_export(&mut original);
    drop(original);

    let mut index = corrupt_and_rebuild(dir.path(), NOW + 20);
    let conn = index.conn();
    assert_eq!(
        read_floor(conn).unwrap(),
        2,
        "the floor restored as a raise"
    );
    assert_eq!(xp_rows(conn, "project_id IS NULL"), before.award);
    assert_eq!(xp_rows(conn, "track = 'git'"), before.frozen);
    assert_eq!(
        git_owners(conn),
        [frozen.0, frozen.0],
        "the frozen project keeps its id"
    );
    let after = index.with_tx(|tx| read_level(tx, NOW + 30)).unwrap();
    eprintln!("level before {:?}, after {:?}", before.level, after.level);
    assert!(after.level >= before.level, "the rebuild lowered the level");

    let gate = unclassified(index.conn()).unwrap_or_else(|findings| panic!("{findings:#?}"));
    eprintln!(
        "registration gate: {} tables and {} columns classified",
        gate.tables, gate.columns
    );
    assert_ne!(gate.tables, 0, "the gate saw no table");

    the_kept_project_returns(&mut index, &before);
    a_lower_floor_changes_nothing(&mut index, &doc);
    the_legacy_key_restores_no_floor();
}

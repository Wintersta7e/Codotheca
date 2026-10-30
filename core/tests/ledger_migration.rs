//! `0019_ledger.sql` — the `xp_events` rebuild for §38's seven kinds, the undeletable session
//! track, the floor table, the completion backfill and the J4 re-queue (§38.13).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::clock::local_date;
use codotheca_core::index::migrate::{apply_all, MIGRATIONS, SUPPORTED_SCHEMA_VERSION};
use codotheca_core::index::subject::ProjectSubject;
use codotheca_core::index::{open_connection, Index, IndexError};
use codotheca_core::ledger::{CUT_KINDS, GIT_KINDS, KINDS, LEGACY_FLOOR_KEY, RETAINED_KINDS};
use rusqlite::Connection;

/// The version this file's migration brings the database to.
const LEDGER: usize = 19;

/// The schema before this file's migration.
const BEFORE: usize = 18;

const SQL: &str = include_str!("../migrations/0019_ledger.sql");

/// A database at exactly `version` files applied, through the shipped set.
fn migrated_to(version: usize) -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    let chain = MIGRATIONS
        .get(..version)
        .unwrap_or_else(|| panic!("the shipped chain does not reach {version}"));
    apply_all(&mut conn, chain).unwrap();
    (dir, conn)
}

/// Brings a database built by `migrated_to(BEFORE)` across this file's migration.
fn upgrade(conn: &mut Connection) {
    let chain = MIGRATIONS
        .get(..LEDGER)
        .unwrap_or_else(|| panic!("the shipped chain does not reach {LEDGER}"));
    assert_eq!(apply_all(conn, chain).unwrap(), 19);
}

fn insert_project(conn: &Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES (?1, ?1, 1, 1)",
        [name],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// A project the backfill's gate admits on everything but its commit-day count.
fn authored(conn: &Connection, name: &str, lineage: &str, remote: Option<&str>) -> i64 {
    let id = insert_project(conn, name);
    conn.execute(
        "UPDATE project SET lineage_key = ?2, remote_key = ?3, authored_by_user = 1
          WHERE id = ?1",
        rusqlite::params![id, lineage, remote],
    )
    .unwrap();
    id
}

fn xp_insert(
    conn: &Connection,
    project: i64,
    kind: &str,
    track: &str,
    dedupe: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                track, meta)
         VALUES (1, 0, ?1, 'lineage:k|remote:', ?2, ?3, ?4, NULL)",
        rusqlite::params![project, kind, dedupe, track],
    )
}

fn commit_days(conn: &Connection, project: i64, n: i64) {
    for day in 0..n {
        xp_insert(
            conn,
            project,
            "commit_day",
            "git",
            &format!("commit_day:{project}:{day}"),
        )
        .unwrap();
    }
}

fn check(conn: &Connection, project: i64, key: &str, state: &str, observed_at: i64) {
    let reason = (state == "unknown").then_some("notRunYet");
    conn.execute(
        "INSERT INTO project_check (project_id, check_key, state, unknown_reason, observed_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![project, key, state, reason, observed_at],
    )
    .unwrap();
}

fn xp_ddl(conn: &Connection) -> String {
    conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'xp_events'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

/// Every quoted literal in the `IN (…)` list that follows `opener` in one table's DDL.
fn in_list(ddl: &str, opener: &str) -> Vec<String> {
    let start = ddl
        .find(opener)
        .unwrap_or_else(|| panic!("no `{opener}` in {ddl}"))
        + opener.len();
    let body = ddl.get(start..).expect("the opener ends inside the DDL");
    let (list, _) = body.split_once(')').expect("the IN list closes");
    list.split(',')
        .map(|v| v.trim().trim_matches('\'').to_owned())
        .collect()
}

#[test]
fn the_chain_reaches_nineteen_and_the_file_is_a_rebuild() {
    let migration = MIGRATIONS
        .get(LEDGER - 1)
        .expect("the shipped chain reaches 19");
    eprintln!(
        "migration 19: {} (rebuild: {})",
        migration.name, migration.rebuilds_a_table
    );
    assert_eq!(migration.version, 19);
    assert_eq!(migration.name, "ledger");
    assert!(
        migration.rebuilds_a_table,
        "0019 drops and renames xp_events, so the runner must toggle foreign keys around it"
    );
    assert_eq!(SUPPORTED_SCHEMA_VERSION, MIGRATIONS.last().unwrap().version);
}

/// **`AC-P4-38-1`.** One row per kind lands on its class's track and is refused on the other; every
/// cut kind is refused on both; and the DDL's two lists are the module's, read back out of
/// `sqlite_master` rather than restated, so neither copy can drift alone.
#[test]
fn ac_p4_38_1_kind_check() {
    let (_d, conn) = migrated_to(LEDGER);
    let p = insert_project(&conn, "thing");
    let mut inserted = 0_usize;
    for (i, kind) in KINDS.iter().enumerate() {
        let (track, other) = if RETAINED_KINDS.contains(kind) {
            ("session", "git")
        } else {
            ("git", "session")
        };
        xp_insert(&conn, p, kind, track, &format!("ok:{i}"))
            .unwrap_or_else(|e| panic!("{kind} on the {track} track: {e}"));
        inserted += 1;
        assert!(
            xp_insert(&conn, p, kind, other, &format!("other:{i}")).is_err(),
            "{kind} must not be storable on the {other} track"
        );
    }
    eprintln!("AC-P4-38-1 kinds inserted: {inserted}");
    assert_eq!(inserted, 7);

    for (i, kind) in CUT_KINDS.iter().enumerate() {
        for track in ["git", "session"] {
            assert!(
                xp_insert(&conn, p, kind, track, &format!("cut:{i}:{track}")).is_err(),
                "the cut kind {kind} must not be storable on the {track} track"
            );
        }
    }

    let ddl = xp_ddl(&conn);
    assert_eq!(in_list(&ddl, "CHECK (kind IN ("), KINDS);
    assert_eq!(
        in_list(&ddl, "CHECK ((track = 'session') = (kind IN ("),
        RETAINED_KINDS
    );
    let classes: Vec<&str> = GIT_KINDS.iter().chain(&RETAINED_KINDS).copied().collect();
    assert_eq!(
        classes, KINDS,
        "the two classes partition the kinds, in order"
    );
}

/// One `xp_events` row, all nine columns.
type XpRow = (
    i64,
    i64,
    Option<i64>,
    Option<i64>,
    String,
    String,
    String,
    String,
    Option<String>,
);

fn xp_rows(conn: &Connection) -> Vec<XpRow> {
    conn.prepare(
        "SELECT id, ts, tz_offset_min, project_id, subject_key, kind, dedupe_key, track, meta
           FROM xp_events ORDER BY id",
    )
    .unwrap()
    .query_map([], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
            r.get(7)?,
            r.get(8)?,
        ))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

/// **`AC-P4-38-2`.** The rebuild copies every surviving row byte for byte, keeps the autoincrement
/// mark a deleted row left, recreates the index, and executes no pragma. A stored row of a cut
/// kind fails the migration and the open puts the backup back.
#[test]
fn ac_p4_38_2_the_rebuild_keeps_what_it_copies() {
    let (_d, mut conn) = migrated_to(BEFORE);
    let p = insert_project(&conn, "kept");
    conn.execute(
        "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                track, meta)
         VALUES (100, -300, ?1, 'lineage:a|remote:', 'commit_day', 'commit_day:a::1', 'git',
                 NULL),
                (200, NULL, ?1, 'lineage:a|remote:', 'commit_day', 'commit_day:a::2', 'git',
                 '{\"n\":1}'),
                (300, 60, ?1, 'lineage:a|remote:', 'debt_day', 'debt_day:a:3', 'session',
                 '{\"what\":\"x\"}'),
                (400, 0, ?1, 'lineage:a|remote:', 'debt_day', 'debt_day:a:4', 'session', NULL)",
        [p],
    )
    .unwrap();
    let highest: i64 = conn
        .query_row("SELECT max(id) FROM xp_events", [], |r| r.get(0))
        .unwrap();
    conn.execute("DELETE FROM xp_events WHERE id = ?1", [highest])
        .unwrap();
    let before = xp_rows(&conn);

    upgrade(&mut conn);

    let after = xp_rows(&conn);
    eprintln!(
        "AC-P4-38-2 rows copied: {} of {}",
        after.len(),
        before.len()
    );
    assert_eq!(before.len(), 3);
    assert_eq!(after, before, "every column of every row survives the copy");

    xp_insert(&conn, p, "focus", "session", "focus:new").unwrap();
    assert!(
        conn.last_insert_rowid() > highest,
        "a new row reused the deleted id {highest}: the high-water mark was lowered"
    );
    let indexes: Vec<String> = conn
        .prepare(
            "SELECT name FROM sqlite_master
              WHERE type = 'index' AND tbl_name = 'xp_events' AND sql IS NOT NULL",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        indexes,
        ["idx_xp_events_project_kind_ts"],
        "the rebuild's one index, serving every `project_id`-prefix read"
    );

    // A pragma inside the runner's transaction is a no-op that reads as if it worked. The header
    // explains that by naming the pragma, so comments come off first.
    let statements: String = SQL
        .lines()
        .map(|line| line.split("--").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    eprintln!(
        "0019_ledger.sql: {} bytes of statement text",
        statements.len()
    );
    assert_ne!(statements.trim(), "");
    assert!(
        !statements.to_uppercase().contains("PRAGMA"),
        "0019_ledger.sql executes a PRAGMA"
    );

    a_cut_kind_fails_the_migration_and_the_backup_is_restored();
}

/// The query plan of `sql`, one detail per step.
fn plan(conn: &Connection, sql: &str) -> String {
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    let rows = stmt.query_map([], |r| r.get::<_, String>(3)).unwrap();
    rows.map(Result::unwrap).collect::<Vec<_>>().join(" | ")
}

/// The ledger's readers that filter one project's rows by kind, and the session total that reads
/// one session's segments, each find their rows through an index rather than a scan.
#[test]
fn the_kind_filtered_readers_use_an_index() {
    let (_d, conn) = migrated_to(MIGRATIONS.len());
    let kind_index = "idx_xp_events_project_kind_ts";
    let cases = [
        (
            kind_index,
            "SELECT EXISTS (SELECT 1 FROM xp_events WHERE project_id = 1 AND kind = 'clean_archive')",
        ),
        (
            kind_index,
            "SELECT id FROM xp_events
              WHERE project_id = 1 AND kind = 'completion_check'
                AND json_extract(meta, '$.what') = 'readme'
              ORDER BY id LIMIT 1",
        ),
        (
            kind_index,
            "SELECT id FROM xp_events
              WHERE project_id = 1 AND kind = 'debt_day'
                AND COALESCE(json_extract(meta, '$.when'),
                             date(ts + COALESCE(tz_offset_min, 0) * 60, 'unixepoch')) = '2027-01-01'
              ORDER BY id LIMIT 1",
        ),
        (
            kind_index,
            "SELECT COUNT(*) FROM xp_events
              WHERE project_id = 1 AND kind = 'commit_day' AND ts >= 0 AND ts < 86400",
        ),
        (
            kind_index,
            "SELECT p.is_archived = 1
                    AND NOT EXISTS (SELECT 1 FROM xp_events x
                                     WHERE x.project_id = p.id AND x.kind = 'clean_archive')
               FROM project p WHERE p.id = 1",
        ),
        (
            "idx_session_segment_session",
            "SELECT COALESCE(SUM(credited_seconds), 0) FROM session_segment WHERE session_id = 1",
        ),
        (
            "idx_session_segment_session",
            "SELECT id FROM session_segment WHERE session_id = 1 ORDER BY started_at, id",
        ),
    ];
    for (index, sql) in cases {
        let detail = plan(&conn, sql);
        eprintln!(
            "{detail}\n  <- {}",
            sql.split_whitespace().collect::<Vec<_>>().join(" ")
        );
        assert!(detail.contains(index), "{index} unused; plan was: {detail}");
    }
}

/// The second fixture of `AC-P4-38-2`, opened through the production door so the backup is real.
fn a_cut_kind_fails_the_migration_and_the_backup_is_restored() {
    let (dir, conn) = migrated_to(BEFORE);
    let p = insert_project(&conn, "cut");
    xp_insert(&conn, p, "revival", "git", "revival:a").unwrap();
    drop(conn);

    match Index::open(dir.path()) {
        Err(IndexError::MigrationFailed {
            version,
            restored_to,
            ..
        }) => {
            assert_eq!(version, 19);
            assert_eq!(restored_to, 18);
        }
        Err(other) => panic!("expected the migration to fail, got {other}"),
        Ok(_) => panic!("a stored revival row was migrated"),
    }

    let reopened = open_connection(&Index::db_path(dir.path())).unwrap();
    let version: u32 = reopened
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    let kinds: Vec<String> = reopened
        .prepare("SELECT kind FROM xp_events")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    eprintln!("after the failed open: version {version}, rows {kinds:?}");
    assert_eq!(version, 18);
    assert_eq!(kinds, ["revival"]);
}

fn floor(conn: &Connection) -> Vec<(i64, i64, Option<i64>)> {
    conn.prepare("SELECT id, level, raised_at FROM level_floor ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// **`AC-P4-38-4`.** One row, seeded at level 1, that only ever rises: every way to add, replace,
/// remove, lower or re-key it aborts. The legacy count is gone.
#[test]
fn ac_p4_38_4_the_floor_table() {
    let (_d, mut conn) = migrated_to(BEFORE);
    conn.execute(
        "INSERT INTO app_meta (k, v) VALUES (?1, '4200')",
        [LEGACY_FLOOR_KEY],
    )
    .unwrap();
    upgrade(&mut conn);

    assert_eq!(floor(&conn), [(1, 1, None)]);
    conn.execute(
        "UPDATE level_floor SET level = 5, raised_at = 100 WHERE id = 1",
        [],
    )
    .unwrap();
    assert_eq!(floor(&conn), [(1, 5, Some(100))], "a raise succeeds");

    let refused = [
        "INSERT INTO level_floor VALUES (2, 9, 200)",
        "INSERT OR REPLACE INTO level_floor VALUES (1, 2, 200)",
        "INSERT INTO level_floor VALUES (1, 9, 200)
           ON CONFLICT (id) DO UPDATE SET level = excluded.level, raised_at = excluded.raised_at",
        "DELETE FROM level_floor",
        "UPDATE level_floor SET level = 4 WHERE id = 1",
        "UPDATE level_floor SET id = 2 WHERE id = 1",
    ];
    for sql in refused {
        assert!(conn.execute(sql, []).is_err(), "not refused: {sql}");
        assert_eq!(floor(&conn), [(1, 5, Some(100))], "changed by: {sql}");
    }
    eprintln!("AC-P4-38-4 writes refused: {}", refused.len());

    let legacy: i64 = conn
        .query_row(
            "SELECT count(*) FROM app_meta WHERE k = ?1",
            [LEGACY_FLOOR_KEY],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(legacy, 0, "the legacy floor count survived the migration");
}

/// One backfilled latch as the test reads it back.
#[derive(Debug, PartialEq, Eq)]
struct Latch {
    project_id: i64,
    ts: i64,
    tz_offset_min: Option<i64>,
    subject_key: String,
    dedupe_key: String,
    track: String,
    meta: serde_json::Value,
}

fn latches(conn: &Connection) -> Vec<Latch> {
    conn.prepare(
        "SELECT project_id, ts, tz_offset_min, subject_key, dedupe_key, track, meta
           FROM xp_events WHERE kind = 'completion_check' ORDER BY dedupe_key",
    )
    .unwrap()
    .query_map([], |r| {
        let meta: String = r.get(6)?;
        Ok(Latch {
            project_id: r.get(0)?,
            ts: r.get(1)?,
            tz_offset_min: r.get(2)?,
            subject_key: r.get(3)?,
            dedupe_key: r.get(4)?,
            track: r.get(5)?,
            meta: serde_json::from_str(&meta).unwrap(),
        })
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

fn expected_latch(
    project_id: i64,
    name: &str,
    subject: &ProjectSubject,
    key: &str,
    observed_at: i64,
) -> Latch {
    let subject_key = subject.to_key();
    Latch {
        project_id,
        ts: observed_at,
        tz_offset_min: None,
        dedupe_key: format!("completion_check:{subject_key}:{key}"),
        subject_key,
        track: "session".to_owned(),
        meta: serde_json::json!({
            "what": key,
            "when": local_date(observed_at.div_euclid(86_400)),
            "project_name": name,
            "backfilled": true,
        }),
    }
}

/// **`AC-P4-38-17`.** Every stored `pass` of a gated project becomes one latch dated when the pass
/// was established; the gate's every refusal writes nothing.
///
/// No `completion_check` row can exist at version 18 — its kind CHECK refuses one — so the only
/// latch already present when the backfill writes is one it has just written for another live
/// project holding the same subject. That is the case the last project exercises.
#[test]
fn ac_p4_38_17_the_backfill() {
    let (_d, mut conn) = migrated_to(BEFORE);
    let remote = Some("github.example/owner/a");
    let a = authored(&conn, "alpha", "aaaa", remote);
    commit_days(&conn, a, 3);
    check(&conn, a, "readme", "pass", 1_700_000_000);
    check(&conn, a, "license", "pass", 1_700_100_000);
    check(&conn, a, "tests", "fail", 1_700_200_000);
    check(&conn, a, "ci", "unknown", 1_700_300_000);

    let b = authored(&conn, "beta", "bbbb", None);
    commit_days(&conn, b, 4);
    check(&conn, b, "remote", "pass", 1_650_000_000);

    let two_days = authored(&conn, "two-days", "cccc", None);
    commit_days(&conn, two_days, 2);
    check(&conn, two_days, "readme", "pass", 1_700_000_000);

    let reference = authored(&conn, "reference", "dddd", None);
    conn.execute(
        "UPDATE project SET is_reference = 1 WHERE id = ?1",
        [reference],
    )
    .unwrap();
    commit_days(&conn, reference, 3);
    check(&conn, reference, "readme", "pass", 1_700_000_000);

    let unclassified = authored(&conn, "unclassified", "eeee", None);
    conn.execute(
        "UPDATE project SET authored_by_user = NULL WHERE id = ?1",
        [unclassified],
    )
    .unwrap();
    commit_days(&conn, unclassified, 3);
    check(&conn, unclassified, "readme", "pass", 1_700_000_000);

    let merged = authored(&conn, "merged", "ffff", None);
    conn.execute(
        "UPDATE project SET merged_into = ?2 WHERE id = ?1",
        [merged, b],
    )
    .unwrap();
    commit_days(&conn, merged, 3);
    check(&conn, merged, "readme", "pass", 1_700_000_000);

    // A second live project on alpha's subject: its `readme` pass names alpha's latch.
    let twin = authored(&conn, "alpha-twin", "aaaa", remote);
    commit_days(&conn, twin, 3);
    check(&conn, twin, "readme", "pass", 1_710_000_000);

    upgrade(&mut conn);

    let alpha = ProjectSubject::Lineage {
        lineage_key: "aaaa".to_owned(),
        remote_key: remote.map(str::to_owned),
    };
    let beta = ProjectSubject::Lineage {
        lineage_key: "bbbb".to_owned(),
        remote_key: None,
    };
    let got = latches(&conn);
    eprintln!("AC-P4-38-17 latches backfilled: {}", got.len());
    assert_ne!(got.len(), 0, "the seeded fixture backfilled nothing");
    assert_eq!(
        got,
        [
            expected_latch(a, "alpha", &alpha, "license", 1_700_100_000),
            expected_latch(a, "alpha", &alpha, "readme", 1_700_000_000),
            expected_latch(b, "beta", &beta, "remote", 1_650_000_000),
        ]
    );
}

/// One `project_job_state` row, every column.
type JobRow = (
    i64,
    String,
    String,
    i64,
    Option<String>,
    i64,
    Option<String>,
    Option<i64>,
    Option<i64>,
);

fn job_rows(conn: &Connection) -> Vec<JobRow> {
    conn.prepare(
        "SELECT project_id, job, state, fail_count, reason, at, cursor, progress_done,
                progress_total
           FROM project_job_state ORDER BY project_id, job",
    )
    .unwrap()
    .query_map([], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
            r.get(7)?,
            r.get(8)?,
        ))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

/// Every J4 `ok` is queued again so the new kinds are derived, with `at` as it stood; no other job
/// and no other state moves.
#[test]
fn the_upgrade_requeues_every_ok_history_and_keeps_at() {
    let (_d, mut conn) = migrated_to(BEFORE);
    let one = insert_project(&conn, "one");
    let two = insert_project(&conn, "two");
    let three = insert_project(&conn, "three");
    conn.execute(
        "INSERT INTO project_job_state
           (project_id, job, state, fail_count, reason, at, cursor, progress_done, progress_total)
         VALUES (?1, 'j4', 'ok', 0, NULL, 111, NULL, 5, 5),
                (?2, 'j4', 'ok', 1, 'earlier', 222, 'c', NULL, NULL),
                (?1, 'j3', 'ok', 0, NULL, 333, NULL, NULL, NULL),
                (?3, 'j4', 'failed', 2, 'git_failed', 444, NULL, NULL, NULL),
                (?3, 'j1', 'queued', 0, NULL, 555, NULL, NULL, NULL)",
        [one, two, three],
    )
    .unwrap();
    let before = job_rows(&conn);

    upgrade(&mut conn);

    let after = job_rows(&conn);
    let requeued = |row: &JobRow| {
        let mut row = row.clone();
        row.2 = "queued".to_owned();
        row.4 = Some("app_upgraded".to_owned());
        row
    };
    let expected: Vec<JobRow> = before
        .iter()
        .map(|row| {
            if row.1 == "j4" && row.2 == "ok" {
                requeued(row)
            } else {
                row.clone()
            }
        })
        .collect();
    let moved = after
        .iter()
        .filter(|r| r.4.as_deref() == Some("app_upgraded"));
    eprintln!("J4 histories re-queued: {}", moved.count());
    assert_eq!(after, expected);
}

/// The retained half is a record history cannot reproduce, so the schema refuses its deletion for
/// every retained kind; the git half stays deletable, because recomputing it is how it is kept.
#[test]
fn a_session_row_cannot_be_deleted() {
    let (_d, conn) = migrated_to(LEDGER);
    let p = insert_project(&conn, "kept");
    for (i, kind) in RETAINED_KINDS.iter().enumerate() {
        xp_insert(&conn, p, kind, "session", &format!("kept:{i}")).unwrap();
        let id = conn.last_insert_rowid();
        assert!(
            conn.execute("DELETE FROM xp_events WHERE id = ?1", [id])
                .is_err(),
            "a {kind} row was deleted"
        );
    }
    let kept: i64 = conn
        .query_row(
            "SELECT count(*) FROM xp_events WHERE track = 'session'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    eprintln!("session rows kept against a delete: {kept}");
    assert_eq!(kept, 5);

    xp_insert(&conn, p, "commit_day", "git", "commit_day:gone").unwrap();
    let deleted = conn
        .execute("DELETE FROM xp_events WHERE track = 'git'", [])
        .unwrap();
    assert_eq!(deleted, 1, "a git row is recomputable and stays deletable");
}

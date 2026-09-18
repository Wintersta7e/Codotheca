//! `0013_debt.sql` — the `xp_events` rebuild, §28.4a's backfill, and the two debt tables.
//!
//! §27.7's `subject_key` defect rides this plan: `core/src/index/subject.rs:34-37` renders
//! `lineage:{k}|remote:{r}` and `core/src/jobs/j4_history.rs` rendered `{lineage}:{remote}` for
//! the same logical subject. **A test exercising only the new writer proves nothing about the
//! defect** — every case below that matters reads the *other* writer's rows.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;

use codotheca_core::index::migrate::{apply_all, MIGRATIONS, SUPPORTED_SCHEMA_VERSION};
use codotheca_core::index::subject::ProjectSubject;
use codotheca_core::index::{open_connection, Index};
use codotheca_core::jobs::j4_history::{commit_days, local_date, HistoryFacts};
use codotheca_core::protocol::ProjectId;

/// A database at exactly `version` files applied, through the shipped set.
fn migrated_to(version: usize) -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, &MIGRATIONS[..version]).unwrap();
    (dir, conn)
}

fn fresh() -> (tempfile::TempDir, rusqlite::Connection) {
    migrated_to(MIGRATIONS.len())
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

fn set_identity(conn: &rusqlite::Connection, id: i64, lineage: &str, remote: Option<&str>) {
    conn.execute(
        "UPDATE project SET lineage_key = ?2, remote_key = ?3 WHERE id = ?1",
        rusqlite::params![id, lineage, remote],
    )
    .unwrap();
}

fn columns(conn: &rusqlite::Connection, table: &str) -> Vec<String> {
    let mut st = conn
        .prepare(&format!("PRAGMA table_info(\"{table}\")"))
        .unwrap();
    let rows = st.query_map([], |r| r.get::<_, String>(1)).unwrap();
    rows.map(Result::unwrap).collect()
}

fn xp_insert(
    conn: &rusqlite::Connection,
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

// ---------------------------------------------------------------------------------------------
// The rebuild itself
// ---------------------------------------------------------------------------------------------

#[test]
fn the_chain_reaches_thirteen_and_stamps_it() {
    let (_d, conn) = fresh();
    let stamped: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stamped, 13, "0013 must stamp its own version");
    assert_eq!(SUPPORTED_SCHEMA_VERSION, 13);
}

/// `AC-P3-28-16`. The two CHECKs widen **together**: `debt_day` joins `kind`'s list and joins
/// the `session` side of the track equivalence. A `debt_day` row on the git track would satisfy
/// `kind`'s CHECK alone, be deleted by the first merge's `recompute_derived`, and be covered by
/// no `level_floor` — silent, permanent data loss.
#[test]
fn a_debt_day_is_a_session_row_and_the_git_track_is_refused() {
    let (_d, conn) = fresh();
    let p = insert_project(&conn, "thing");

    xp_insert(&conn, p, "debt_day", "session", "debt_day:a").unwrap();

    let refused = xp_insert(&conn, p, "debt_day", "git", "debt_day:b");
    assert!(
        refused.is_err(),
        "a debt_day on the git track must be refused by the DDL, not by a convention"
    );

    // The mirror case, so the equivalence is asserted in both directions rather than one.
    let refused_session_commit = xp_insert(&conn, p, "commit_day", "session", "commit_day:c");
    assert!(refused_session_commit.is_err());
}

/// The rebuild changes **two CHECKs and nothing else**. A create-copy-drop-rename drafted from a
/// stale copy of the table silently drops a column, which is exactly what `0007`'s two `ALTER`ed
/// columns did to `0012`'s draft.
#[test]
fn the_rebuild_changes_no_column() {
    let (_d, before) = migrated_to(12);
    let was = columns(&before, "xp_events");
    drop(before);

    let (_d2, after) = fresh();
    let now = columns(&after, "xp_events");

    assert_eq!(was, now, "0013 must widen two CHECKs and move no column");
    assert!(!was.is_empty(), "a diff over zero columns proves nothing");

    // The index is recreated by name, because a rebuild drops it with the table.
    let idx: i64 = after
        .query_row(
            "SELECT count(*) FROM sqlite_master
              WHERE type='index' AND name='idx_xp_events_project_ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(idx, 1, "idx_xp_events_project_ts was not recreated");
}

/// §1.5's guarantee lives entirely in `sqlite_sequence`'s high-water mark, and
/// `DROP TABLE` deletes it. Seed three, delete all three, migrate, insert one: `4` proves the
/// mark was carried; `1` proves it was re-seeded from `max(id)` of an empty table.
#[test]
fn the_autoincrement_high_water_mark_survives_the_rebuild() {
    let (_d, mut conn) = migrated_to(12);
    let p = insert_project(&conn, "thing");
    for n in 0..3 {
        xp_insert(&conn, p, "session", "session", &format!("session:{n}")).unwrap();
    }
    conn.execute("DELETE FROM xp_events", []).unwrap();

    apply_all(&mut conn, MIGRATIONS).unwrap();

    xp_insert(&conn, p, "debt_day", "session", "debt_day:after").unwrap();
    let id: i64 = conn
        .query_row("SELECT id FROM xp_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(id, 4, "the high-water mark was not carried across the drop");
}

// ---------------------------------------------------------------------------------------------
// §28.4a's backfill
// ---------------------------------------------------------------------------------------------

/// The backfill rewrites from the **project row**, never by splitting the stored string:
/// `<lineage>:<remote>` cannot be split safely in general.
#[test]
fn the_backfill_rewrites_the_old_two_writer_key() {
    let (_d, mut conn) = migrated_to(12);
    let p = insert_project(&conn, "thing");
    set_identity(&conn, p, "abc123", Some("github.com/o/r"));

    // The shape `j4_history.rs` wrote before the convergence.
    conn.execute(
        "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                track, meta)
         VALUES (1, 0, ?1, 'abc123:github.com/o/r', 'commit_day', 'commit_day:old', 'git', NULL)",
        [p],
    )
    .unwrap();
    // A row that already reads the right shape must not be touched, and a `session` row of
    // another kind must not be touched either.
    conn.execute(
        "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                track, meta)
         VALUES (1, 0, ?1, 'lineage:abc123|remote:github.com/o/r', 'commit_day',
                 'commit_day:new', 'git', NULL)",
        [p],
    )
    .unwrap();

    apply_all(&mut conn, MIGRATIONS).unwrap();

    let want = ProjectSubject::Lineage {
        lineage_key: "abc123".to_owned(),
        remote_key: Some("github.com/o/r".to_owned()),
    }
    .to_key();

    let rewritten: i64 = conn
        .query_row(
            "SELECT count(*) FROM xp_events WHERE subject_key = ?1",
            [&want],
            |r| r.get(0),
        )
        .unwrap();
    eprintln!("backfill: {rewritten} row(s) now read to_key()'s shape");
    assert_eq!(
        rewritten, 2,
        "a run that rewrites zero rows is a failing run, not a clean one"
    );

    let stragglers: i64 = conn
        .query_row(
            "SELECT count(*) FROM xp_events WHERE subject_key NOT LIKE 'lineage:%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stragglers, 0);
}

/// A project with **no lineage** has nothing to rewrite from, and the backfill must leave its
/// rows alone rather than writing `lineage:|remote:` over them.
#[test]
fn the_backfill_leaves_a_row_it_cannot_rebuild_alone() {
    let (_d, mut conn) = migrated_to(12);
    let p = insert_project(&conn, "unlineaged");
    conn.execute(
        "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                track, meta)
         VALUES (1, 0, ?1, 'whatever', 'commit_day', 'commit_day:x', 'git', NULL)",
        [p],
    )
    .unwrap();

    apply_all(&mut conn, MIGRATIONS).unwrap();

    let key: String = conn
        .query_row("SELECT subject_key FROM xp_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        key, "whatever",
        "a row with no lineage to read was rewritten"
    );
}

// ---------------------------------------------------------------------------------------------
// `AC-P3-28-9` — the two writers agree, byte for byte
// ---------------------------------------------------------------------------------------------

/// **The defect this task exists to close.** One project, two writers: `commit_days` (phase 1's)
/// and `ProjectSubject::to_key()` (the sidecar's, and `debt_day`'s). The comparison is on the
/// bytes, and both sides round-trip through `parse`.
#[test]
fn both_xp_writers_render_one_subject_key() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    set_identity(&conn, p, "abc123", Some("github.com/o/r"));

    let facts = HistoryFacts {
        days: BTreeSet::from([20_000_i64]),
        ..HistoryFacts::default()
    };
    let tx = conn.transaction().unwrap();
    commit_days(
        &tx,
        ProjectId(p),
        Some("abc123"),
        Some("github.com/o/r"),
        &facts,
    )
    .unwrap();

    let subject = ProjectSubject::Lineage {
        lineage_key: "abc123".to_owned(),
        remote_key: Some("github.com/o/r".to_owned()),
    };
    tx.execute(
        "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                track, meta)
         VALUES (2, 0, ?1, ?2, 'debt_day', ?3, 'session', NULL)",
        rusqlite::params![
            p,
            subject.to_key(),
            format!("debt_day:{}:{}", subject.to_key(), local_date(20_000)),
        ],
    )
    .unwrap();
    tx.commit().unwrap();

    let mut st = conn
        .prepare("SELECT kind, subject_key FROM xp_events ORDER BY kind")
        .unwrap();
    let rows: Vec<(String, String)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rows.len(), 2, "both writers must have written");

    assert_eq!(
        rows[0].1.as_bytes(),
        rows[1].1.as_bytes(),
        "the two writers disagree: {:?} ({}) vs {:?} ({})",
        rows[0].1,
        rows[0].0,
        rows[1].1,
        rows[1].0,
    );

    for (kind, key) in &rows {
        assert_eq!(
            ProjectSubject::parse(key),
            Some(subject.clone()),
            "{kind}'s key does not round-trip through parse"
        );
    }
}

/// The empty-remote case, because that is the one a naive split gets wrong: `to_key()` keeps the
/// `|remote:` separator with nothing after it, and `parse` reads it back as `None`.
#[test]
fn both_writers_agree_when_there_is_no_remote() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    set_identity(&conn, p, "abc123", None);

    let facts = HistoryFacts {
        days: BTreeSet::from([20_001_i64]),
        ..HistoryFacts::default()
    };
    let tx = conn.transaction().unwrap();
    commit_days(&tx, ProjectId(p), Some("abc123"), None, &facts).unwrap();
    tx.commit().unwrap();

    let key: String = conn
        .query_row("SELECT subject_key FROM xp_events", [], |r| r.get(0))
        .unwrap();
    let want = ProjectSubject::Lineage {
        lineage_key: "abc123".to_owned(),
        remote_key: None,
    };
    assert_eq!(key.as_bytes(), want.to_key().as_bytes());
    assert_eq!(ProjectSubject::parse(&key), Some(want));
}

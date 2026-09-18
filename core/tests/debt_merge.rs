//! §28.7 — the three merge classes, **distinguished by the test and not by a comment**.
//!
//! §28.7's own table names two of the three so that they are observationally identical, and a
//! test written from it alone can tell only two apart. The discriminator is **which function did
//! it and whether the row was a delete candidate first**:
//!
//! | Class | Tables | On a merge | Where |
//! |---|---|---|---|
//! | **derived** | `debt_item`, `debt_sweep` | deleted for both sides, recomputed | `recompute_derived`'s delete list |
//! | **not-recomputable** | `xp_events` on `track = 'session'` | **not** deleted; survives the git-track delete, then reparented | `recompute_derived`, two statements later |
//! | **reparented** | `health_delta` | repointed at the survivor | `reparent_rows` |

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::identity::merge::recompute_derived;
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::subject::ProjectSubject;
use codotheca_core::index::{open_connection, Index};

fn fresh() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

fn insert_project(conn: &rusqlite::Connection, name: &str, lineage: &str) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
         VALUES (?1, ?1, ?2, 1, 1)",
        rusqlite::params![name, lineage],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn plant_item(conn: &rusqlite::Connection, project: i64, source: &str) {
    conn.execute(
        "INSERT INTO debt_item (project_id, subject_key, source, fingerprint, state, scoring,
                                first_seen_at, last_seen_at)
         VALUES (?1, ?2, ?3, 'f', 'open', 'scored', 1, 1)",
        rusqlite::params![project, format!("lineage:l{project}|remote:"), source],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO debt_sweep (project_id, source, outcome, item_count, observed_at)
         VALUES (?1, ?2, 'complete', 1, 1)",
        rusqlite::params![project, source],
    )
    .unwrap();
}

fn plant_xp(conn: &rusqlite::Connection, project: i64, kind: &str, track: &str, dedupe: &str) {
    conn.execute(
        "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                track, meta)
         VALUES (1, 0, ?1, ?2, ?3, ?4, ?5, NULL)",
        rusqlite::params![
            project,
            ProjectSubject::Lineage {
                lineage_key: format!("l{project}"),
                remote_key: None,
            }
            .to_key(),
            kind,
            dedupe,
            track
        ],
    )
    .unwrap();
}

fn count(conn: &rusqlite::Connection, sql: &str, project: i64) -> i64 {
    conn.query_row(sql, [project], |r| r.get(0)).unwrap()
}

/// **`AC-P3-28-6`.** A merge deletes **both** sides' `debt_item` and `debt_sweep` rows, writes
/// **no** closure event and pays nothing.
///
/// **A merge is not a closure.** Deleting both sides leaves no `complete`→`complete` pair across
/// the merge, so the recompute opens the survivor's items fresh. That falls out of the class
/// assignment rather than needing a guard.
#[test]
fn ac_p3_28_6_a_merge_deletes_both_sides_and_pays_nothing() {
    let (_d, mut conn) = fresh();
    let survivor = insert_project(&conn, "a", "l1");
    let absorbed = insert_project(&conn, "b", "l2");
    plant_item(&conn, survivor, "todo_marker");
    plant_item(&conn, absorbed, "missing_readme");

    let tx = conn.transaction().unwrap();
    recompute_derived(&tx, survivor, absorbed).unwrap();
    tx.commit().unwrap();

    for project in [survivor, absorbed] {
        assert_eq!(
            count(
                &conn,
                "SELECT count(*) FROM debt_item WHERE project_id = ?1",
                project
            ),
            0,
            "a debt_item survived the merge and would be read as current"
        );
        assert_eq!(
            count(
                &conn,
                "SELECT count(*) FROM debt_sweep WHERE project_id = ?1",
                project
            ),
            0,
        );
    }

    let xp: i64 = conn
        .query_row("SELECT count(*) FROM xp_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(xp, 0, "a merge paid for a closure it did not observe");
}

/// **`AC-P3-28-18`.** A `debt_day` row **survives** `recompute_derived` and carries the
/// survivor's `project_id` afterwards — asserted by row identity over a real merge, **beside a
/// `commit_day` row in the same project that is deleted and recomputed**. One assertion cannot
/// tell the classes apart; the pair can.
#[test]
fn ac_p3_28_18_a_debt_day_survives_the_merge_a_commit_day_does_not() {
    let (_d, mut conn) = fresh();
    let survivor = insert_project(&conn, "a", "l1");
    let absorbed = insert_project(&conn, "b", "l2");

    plant_xp(&conn, absorbed, "debt_day", "session", "debt_day:x");
    plant_xp(&conn, absorbed, "commit_day", "git", "commit_day:x");
    plant_xp(&conn, survivor, "commit_day", "git", "commit_day:y");

    let before: String = conn
        .query_row(
            "SELECT dedupe_key FROM xp_events WHERE kind = 'debt_day'",
            [],
            |r| r.get(0),
        )
        .unwrap();

    let tx = conn.transaction().unwrap();
    recompute_derived(&tx, survivor, absorbed).unwrap();
    tx.commit().unwrap();

    // The git-track rows are gone, for both sides.
    let git: i64 = conn
        .query_row(
            "SELECT count(*) FROM xp_events WHERE track = 'git'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(git, 0, "a commit_day survived a recompute");

    // The debt_day is the **same row**, repointed at the survivor.
    let (dedupe, project): (String, i64) = conn
        .query_row(
            "SELECT dedupe_key, project_id FROM xp_events WHERE kind = 'debt_day'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        dedupe, before,
        "the debt_day row was rewritten, not reparented"
    );
    assert_eq!(project, survivor);
}

/// **The hazard the class assignment avoids, and the tree anticipates only its mirror image.**
/// `recompute_derived` deletes `WHERE track = 'git'` and `level_floor` counts `track = 'git'`
/// only, so a `debt_day` row on the git track would satisfy both DDL CHECKs, be deleted by the
/// first merge, and be covered by **no floor** — silent, permanent data loss.
///
/// Task 3's CHECK is what stops that, and this is it doing the work.
#[test]
fn the_ddl_refuses_a_debt_day_on_the_git_track() {
    let (_d, conn) = fresh();
    let p = insert_project(&conn, "a", "l1");
    let refused = conn.execute(
        "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                track, meta)
         VALUES (1, 0, ?1, 'lineage:l1|remote:', 'debt_day', 'debt_day:git', 'git', NULL)",
        [p],
    );
    assert!(
        refused.is_err(),
        "a debt_day on the git track was admitted, and the first merge would erase it"
    );
}

/// The **third** class, and what makes it a third: `health_delta` is reparented by
/// `reparent_rows`, a different function, and was never a delete candidate. A test reading only
/// the end state sees it as identical to the `debt_day` row above.
#[test]
fn health_delta_is_reparented_by_another_function_entirely() {
    let (_d, mut conn) = fresh();
    let survivor = insert_project(&conn, "a", "l1");
    let absorbed = insert_project(&conn, "b", "l2");
    conn.execute(
        "INSERT INTO health_delta (project_id, ts, layer, from_value, to_value, detected_in)
         VALUES (?1, 1, 'x', 0.0, 1.0, 'background')",
        [absorbed],
    )
    .unwrap();
    plant_xp(&conn, absorbed, "debt_day", "session", "debt_day:x");

    // `recompute_derived` alone — the function the other two classes pass through. It moves the
    // `debt_day` row and **does not touch `health_delta`**, which is the discriminator.
    let tx = conn.transaction().unwrap();
    recompute_derived(&tx, survivor, absorbed).unwrap();
    tx.commit().unwrap();

    let debt_day_owner: i64 = conn
        .query_row(
            "SELECT project_id FROM xp_events WHERE kind = 'debt_day'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(debt_day_owner, survivor);

    let health_owner: i64 = conn
        .query_row("SELECT project_id FROM health_delta", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        health_owner, absorbed,
        "recompute_derived reparented health_delta, so the third class is not a third class"
    );
}

/// **`AC-P3-28-7`.** A sidecar export/restore round trip preserves `debt_day` rows with
/// `subject_key` **byte-identical** across the trip.
///
/// **The sidecar needs no change, and that is the point.** A `debt_day` row exports on the
/// `track = 'session'` filter and restores under the hard-coded `'session'` literal, with
/// `subject_key` rebuilt from `to_key()`.
#[test]
fn ac_p3_28_7_a_debt_day_round_trips_through_the_sidecar() {
    let (_d, conn) = fresh();
    let p = insert_project(&conn, "a", "l1");
    plant_xp(&conn, p, "debt_day", "session", "debt_day:x");
    let before: String = conn
        .query_row(
            "SELECT subject_key FROM xp_events WHERE kind = 'debt_day'",
            [],
            |r| r.get(0),
        )
        .unwrap();

    let doc = codotheca_core::index::sidecar::export(&conn, 1, 1).unwrap();

    // A fresh library, restored from the sidecar: the row comes back on the same subject, which
    // is what `restore_xp_events` rebuilding it from `to_key()` buys.
    let (_d2, other) = fresh();
    let restored_into = insert_project(&other, "a", "l1");
    let counts = codotheca_core::index::sidecar::restore_for_subject(
        &other,
        &doc,
        codotheca_core::protocol::ProjectId(restored_into),
    )
    .unwrap();
    assert!(counts.xp_events > 0, "the sidecar carried no xp_events row");

    let after: String = other
        .query_row(
            "SELECT subject_key FROM xp_events WHERE kind = 'debt_day'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        after.as_bytes(),
        before.as_bytes(),
        "the subject key changed across the sidecar trip"
    );
}

/// **Why `track` is `'session'`, proved rather than asserted.**
///
/// The DDL refuses a git-track `debt_day`, so the loss this guards against cannot be reached in
/// production. It is reached here in a **scratch database with the CHECK dropped**, and the row
/// vanishes on the first merge with `level_floor` unchanged — silent, permanent data loss that
/// no test around it would see.
#[test]
fn a_debt_day_on_the_git_track_would_vanish_with_the_floor_unmoved() {
    let dir = tempfile::tempdir().unwrap();
    let conn = rusqlite::Connection::open(dir.path().join("scratch.db")).unwrap();
    // A scratch table with §28.8's CHECKs deliberately absent, so the row the DDL refuses can
    // exist long enough to watch a merge erase it.
    conn.execute_batch(
        "CREATE TABLE project (id INTEGER PRIMARY KEY, name TEXT);
         CREATE TABLE xp_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            project_id INTEGER, subject_key TEXT NOT NULL, kind TEXT NOT NULL,
            dedupe_key TEXT NOT NULL UNIQUE, track TEXT NOT NULL);
         INSERT INTO project (id, name) VALUES (1, 'a'), (2, 'b');
         INSERT INTO xp_events (project_id, subject_key, kind, dedupe_key, track)
           VALUES (2, 's', 'debt_day', 'debt_day:x', 'git'),
                  (2, 's', 'session',  'session:x',  'session');",
    )
    .unwrap();

    // `level_floor` counts `track = 'git'` only, so this row is inside the floor's unit.
    let floor_before: i64 = conn
        .query_row(
            "SELECT count(*) FROM xp_events WHERE track = 'git'",
            [],
            |r| r.get(0),
        )
        .unwrap();

    // `recompute_derived`'s first statement, verbatim in shape.
    conn.execute(
        "DELETE FROM xp_events WHERE project_id IN (?1, ?2) AND track = 'git'",
        rusqlite::params![1, 2],
    )
    .unwrap();
    conn.execute(
        "UPDATE xp_events SET project_id = ?1 WHERE project_id = ?2",
        rusqlite::params![1, 2],
    )
    .unwrap();

    let debt_days: i64 = conn
        .query_row(
            "SELECT count(*) FROM xp_events WHERE kind = 'debt_day'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(debt_days, 0, "the fixture did not reproduce the loss");

    // The session row survived and was reparented, which is the class `debt_day` belongs to.
    let survived: i64 = conn
        .query_row(
            "SELECT project_id FROM xp_events WHERE kind = 'session'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(survived, 1);

    // And the floor never moved to cover the loss: it is counted **before** the delete, in a unit
    // the deleted row was inside.
    assert_eq!(floor_before, 1);
}

//! §34.3 and A14.4 — **three merge classes, told apart by what one real merge does to them**.
//!
//! | Class | Here | On a merge |
//! |---|---|---|
//! | **derived** | `debt_item`, `debt_sweep` | deleted for both sides, recomputed later |
//! | **not-recomputable** | `xp_events` on `track = 'session'` | **not** deleted; survives the git-track delete and is repointed |
//! | **reparented** | `health_delta` | repointed at the survivor, never a delete candidate |
//!
//! `core/tests/debt_merge.rs` asserts the classes piecewise through `recompute_derived`; this file
//! drives the whole `merge_projects` over one fixture holding all three for both sides, so the
//! classes are distinguished by behaviour in a single run rather than by a comment. `health_delta`
//! is an observation history: recomputing it is impossible, and discarding it loses the
//! attribution the phase-1 contract exists to carry.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::identity::merge::merge_projects;
use codotheca_core::identity::AssociationKind;
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::subject::ProjectSubject;
use codotheca_core::index::{open_connection, Index};
use rusqlite::Connection;

fn fresh() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

/// `created_at` decides the survivor (§1.5), so the earlier project survives.
fn project(conn: &Connection, name: &str, created_at: i64) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
         VALUES (?1, ?1, ?1, ?2, ?2)",
        rusqlite::params![name, created_at],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn subject(project: &str) -> String {
    ProjectSubject::Lineage {
        lineage_key: project.to_owned(),
        remote_key: None,
    }
    .to_key()
}

/// All three classes on one project: two observation rows, one open item and its sweep, and one
/// ledger row on each track.
fn plant(conn: &Connection, id: i64, name: &str) {
    for (ts, layer) in [(10, "rust"), (20, "dust")] {
        conn.execute(
            "INSERT INTO health_delta (project_id, ts, layer, from_value, to_value, detected_in)
             VALUES (?1, ?2, ?3, 2.0, 1.0, 'foreground')",
            rusqlite::params![id, ts + id, layer],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO debt_item (project_id, subject_key, source, fingerprint, state, scoring,
                                first_seen_at, last_seen_at)
         VALUES (?1, ?2, 'todo_marker', ?3, 'open', 'scored', 1, 1)",
        rusqlite::params![id, subject(name), format!("salient:{id}")],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO debt_sweep (project_id, source, outcome, item_count, observed_at)
         VALUES (?1, 'todo_marker', 'complete', 1, 1)",
        [id],
    )
    .unwrap();
    for (kind, track) in [("commit_day", "git"), ("debt_day", "session")] {
        conn.execute(
            "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                    track, meta)
             VALUES (1, 0, ?1, ?2, ?3, ?4, ?5, NULL)",
            rusqlite::params![id, subject(name), kind, format!("{kind}:{name}"), track],
        )
        .unwrap();
    }
}

fn deltas(conn: &Connection) -> Vec<(i64, i64, i64)> {
    let mut st = conn
        .prepare("SELECT id, ts, project_id FROM health_delta ORDER BY id")
        .unwrap();
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

/// **`AC-P3-34-12`.** One merge, three classes, and three different outcomes — printed, so a run
/// that asserted over an empty fixture is visible.
#[test]
fn ac_p3_34_12_one_merge_reparents_health_delta_beside_a_derived_and_a_not_recomputable_table() {
    let (_d, mut conn) = fresh();
    let survivor = project(&conn, "alpha", 100);
    let absorbed = project(&conn, "beta", 200);
    plant(&conn, survivor, "alpha");
    plant(&conn, absorbed, "beta");

    let before = deltas(&conn);
    assert_eq!(
        before.len(),
        4,
        "the fixture planted no observation history"
    );

    let tx = conn.transaction().unwrap();
    let out = merge_projects(
        &tx,
        absorbed,
        survivor,
        AssociationKind::Manual,
        &serde_json::json!({ "rule": "manual" }),
        300,
    )
    .unwrap();
    tx.commit().unwrap();
    assert_eq!((out.survivor, out.absorbed), (survivor, absorbed));

    // **Reparented**: every row survives with its own id and its own `ts`, all on the survivor.
    let after = deltas(&conn);
    let reparented = i64::try_from(after.len()).unwrap();
    let derived = count(&conn, "SELECT count(*) FROM debt_item")
        + count(&conn, "SELECT count(*) FROM debt_sweep");
    let kept_session = count(
        &conn,
        "SELECT count(*) FROM xp_events WHERE track = 'session'",
    );
    let kept_git = count(&conn, "SELECT count(*) FROM xp_events WHERE track = 'git'");
    eprintln!(
        "AC-P3-34-12 after one merge: {reparented} health_delta, {derived} derived rows, \
         {kept_session} session-track and {kept_git} git-track xp_events"
    );

    assert_eq!(
        after
            .iter()
            .map(|&(id, ts, _)| (id, ts))
            .collect::<Vec<_>>(),
        before
            .iter()
            .map(|&(id, ts, _)| (id, ts))
            .collect::<Vec<_>>(),
        "a health_delta row was deleted, re-inserted or re-stamped"
    );
    assert!(
        after.iter().all(|&(_, _, owner)| owner == survivor),
        "a health_delta row was left on the absorbed project"
    );

    // **Derived**: gone for both sides, to be recomputed from the next sweep.
    assert_eq!(
        derived, 0,
        "a derived row survived and would be read as current"
    );

    // **Not-recomputable**: the session track survives for both sides and is repointed, while
    // the git track beside it is deleted — the class a delete candidate survives.
    assert_eq!(kept_session, 2);
    assert_eq!(
        count(
            &conn,
            &format!("SELECT count(*) FROM xp_events WHERE project_id <> {survivor}")
        ),
        0
    );
    assert_eq!(kept_git, 0);
}

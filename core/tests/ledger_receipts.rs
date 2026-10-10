//! §38.2's receipts and lookup: a retained row renders its sentence without a join, and its writer
//! finds the row by `project_id`, never by `dedupe_key` — the subject moves from `path:` to
//! `lineage:` at a first commit, and a key lookup would pay the day twice.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::clock::LocalInstant;
use codotheca_core::debt::identity::DebtKey;
use codotheca_core::debt::store::{DebtCloseReason, DebtClosure, SweepEffect};
use codotheca_core::debt::xp::pay_debt_day;
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::ledger::pending::awaiting_restore;
use codotheca_core::ledger::price::{completion_gate, debt_day_gate, Gate};
use codotheca_core::ledger::receipt::Receipt;
use codotheca_core::protocol::{DebtScoring, DebtSource, ProjectId};
use serde_json::json;

/// 2026-09-17 12:00:00 UTC.
const NOON: i64 = 1_789_646_400;
const NOON_AT_UTC: LocalInstant = LocalInstant {
    unix: NOON,
    offset_min: 0,
};

fn fresh() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

/// An enrolled project with no lineage yet: its subject is its one location's path.
fn insert_unlineaged(conn: &rusqlite::Connection, name: &str) -> ProjectId {
    conn.execute(
        "INSERT INTO project (name, seed_basename, acknowledged_at, created_at, updated_at)
         VALUES (?1, ?1, 1, 1, 1)",
        [name],
    )
    .unwrap();
    let project = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO location (project_id, kind, distro, path_bytes, path_key, path_display,
                               store_key, presence, repo_kind)
         VALUES (?1, 'linux', '', x'2f77', x'2f77', '/w', 'store', 'present', 'worktree')",
        [project],
    )
    .unwrap();
    ProjectId(project)
}

/// An enrolled project with a lineage.
fn insert_lineaged(conn: &rusqlite::Connection, name: &str) -> ProjectId {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, remote_key, acknowledged_at,
                              created_at, updated_at)
         VALUES (?1, ?1, 'abc123', 'github.com/o/r', 1, 1, 1)",
        [name],
    )
    .unwrap();
    ProjectId(conn.last_insert_rowid())
}

fn fixed(n: usize) -> SweepEffect {
    SweepEffect {
        closed: (0..n)
            .map(|i| {
                let mut key = DebtKey::content("subject", &format!("{i:064x}"), 0);
                key.source = DebtSource::TodoMarker;
                DebtClosure {
                    key,
                    reason: DebtCloseReason::Fixed,
                    scoring: DebtScoring::Scored,
                }
            })
            .collect(),
        ..SweepEffect::default()
    }
}

fn pay(conn: &mut rusqlite::Connection, project: ProjectId, n: usize, at: LocalInstant) -> bool {
    let tx = conn.transaction().unwrap();
    let payout = pay_debt_day(&tx, project, &fixed(n), at).unwrap();
    tx.commit().unwrap();
    payout.wrote_row
}

/// Every `debt_day` row: `(ts, tz_offset_min, dedupe_key, meta)`, by id.
fn days(conn: &rusqlite::Connection) -> Vec<(i64, Option<i64>, String, serde_json::Value)> {
    let mut stmt = conn
        .prepare(
            "SELECT ts, tz_offset_min, dedupe_key, meta FROM xp_events
              WHERE kind = 'debt_day' ORDER BY id",
        )
        .unwrap();
    stmt.query_map([], |r| {
        let meta: String = r.get(3)?;
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            serde_json::from_str(&meta).unwrap(),
        ))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

/// An upgraded index holds a day written before receipts: `{sources, closed}` and no `when`. A
/// second closure on the same local date finds it by project and date and updates its `meta`
/// alone, after the subject moved from `path:` to `lineage:` — a key lookup would miss it and pay
/// the day twice.
#[test]
fn a_legacy_day_row_is_found_by_project_and_date() {
    let (_d, mut conn) = fresh();
    let project = insert_unlineaged(&conn, "widget");
    let legacy_key = "debt_day:path:linux::2f77:2026-09-17";
    conn.execute(
        "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                track, meta)
         VALUES (?1, 120, ?2, 'path:linux::2f77', 'debt_day', ?3, 'session',
                 '{\"sources\":[\"todo_marker\"],\"closed\":2}')",
        rusqlite::params![NOON, project.0, legacy_key],
    )
    .unwrap();
    // The first commit: the subject is the lineage from here on.
    conn.execute(
        "UPDATE project SET lineage_key = 'abc123', remote_key = 'github.com/o/r' WHERE id = ?1",
        [project.0],
    )
    .unwrap();

    let wrote = pay(&mut conn, project, 1, NOON_AT_UTC);

    assert!(!wrote, "the day was already paid");
    assert_eq!(
        days(&conn),
        [(
            NOON,
            Some(120),
            legacy_key.to_owned(),
            json!({
                "sources": ["todo_marker"],
                "closed": 3,
                "when": "2026-09-17",
                "project_name": "widget",
            }),
        )]
    );
}

/// A first payout writes the receipt beside the day's sources; a second on the same date sums
/// into it and keeps one row.
#[test]
fn receipts_carry_what_when_project_name() {
    let (_d, mut conn) = fresh();
    let project = insert_lineaged(&conn, "widget");

    assert!(pay(&mut conn, project, 2, NOON_AT_UTC));
    assert!(!pay(&mut conn, project, 1, NOON_AT_UTC));

    let rows = days(&conn);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(
        rows[0].3,
        json!({
            "sources": ["todo_marker"],
            "closed": 3,
            "when": "2026-09-17",
            "project_name": "widget",
        })
    );

    // The other retained kinds take the same three keys from one builder, with their own extras.
    let at = LocalInstant {
        unix: NOON,
        offset_min: -600,
    };
    let receipt: serde_json::Value = serde_json::from_str(
        &Receipt::new("readme", at, "widget")
            .extra("seconds", 1200)
            .to_meta(),
    )
    .unwrap();
    assert_eq!(
        receipt,
        json!({
            "what": "readme",
            "when": "2026-09-17",
            "project_name": "widget",
            "seconds": 1200,
        })
    );
    let late: serde_json::Value = serde_json::from_str(
        &Receipt::new(
            "v1",
            LocalInstant {
                unix: NOON,
                offset_min: 720,
            },
            "widget",
        )
        .to_meta(),
    )
    .unwrap();
    assert_eq!(
        late["when"], "2026-09-18",
        "the date is read in the write's own offset"
    );
}

#[test]
fn silent_is_absent_unless_set() {
    let quiet: serde_json::Value =
        serde_json::from_str(&Receipt::new("v1", NOON_AT_UTC, "w").silent(true).to_meta()).unwrap();
    assert_eq!(quiet["silent"], true);
    for receipt in [
        Receipt::new("v1", NOON_AT_UTC, "w"),
        Receipt::new("v1", NOON_AT_UTC, "w").silent(false),
    ] {
        let meta: serde_json::Value = serde_json::from_str(&receipt.to_meta()).unwrap();
        assert_eq!(meta.get("silent"), None, "{meta}");
    }
}

/// With a restored record still pending and the project's history not concluded, a closure writes
/// nothing: the record may yet match this project and bring its own row back. Once the history
/// job settles, the next closure pays.
#[test]
fn no_retained_write_before_the_match() {
    let (_d, mut conn) = fresh();
    let project = insert_lineaged(&conn, "widget");
    conn.execute(
        "INSERT INTO sidecar_pending (source_generation, subject_key, location_keys, record,
                                     queued_at)
         VALUES (1, 'lineage:other', '[]', '{}', 1)",
        [],
    )
    .unwrap();

    assert!(awaiting_restore(&conn, project).unwrap());
    assert!(!pay(&mut conn, project, 1, NOON_AT_UTC));
    assert_eq!(days(&conn), []);

    conn.execute(
        "INSERT INTO project_job_state (project_id, job, state, at) VALUES (?1, 'j4', 'ok', 1)",
        [project.0],
    )
    .unwrap();
    assert!(!awaiting_restore(&conn, project).unwrap());
    assert!(pay(&mut conn, project, 1, NOON_AT_UTC));
    assert_eq!(days(&conn).len(), 1);
}

/// With nothing pending there is nothing to wait for, concluded or not.
#[test]
fn nothing_pending_awaits_nothing() {
    let (_d, conn) = fresh();
    let project = insert_lineaged(&conn, "widget");
    assert!(!awaiting_restore(&conn, project).unwrap());
}

fn commit_days(conn: &rusqlite::Connection, project: ProjectId, days: std::ops::Range<i64>) {
    for day in days {
        conn.execute(
            "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                    track, meta)
             VALUES (?1, 0, ?2, 'lineage:abc123', 'commit_day', ?3, 'git', '{}')",
            rusqlite::params![day * 86_400, project.0, format!("commit_day:abc123:{day}")],
        )
        .unwrap();
    }
}

#[test]
fn the_gates_read_their_columns() {
    let (_d, conn) = fresh();
    let project = insert_lineaged(&conn, "widget");
    assert_eq!(debt_day_gate(&conn, project).unwrap(), Gate::Open);
    assert_eq!(
        completion_gate(&conn, project).unwrap(),
        Gate::NotComputable,
        "authorship not yet classified"
    );

    conn.execute(
        "UPDATE project SET authored_by_user = 1 WHERE id = ?1",
        [project.0],
    )
    .unwrap();
    commit_days(&conn, project, 0..2);
    assert_eq!(completion_gate(&conn, project).unwrap(), Gate::Shut);
    commit_days(&conn, project, 2..3);
    assert_eq!(completion_gate(&conn, project).unwrap(), Gate::Open);

    conn.execute(
        "UPDATE project SET is_reference = 1 WHERE id = ?1",
        [project.0],
    )
    .unwrap();
    assert_eq!(completion_gate(&conn, project).unwrap(), Gate::Shut);
    assert_eq!(debt_day_gate(&conn, project).unwrap(), Gate::Shut);

    let unenrolled = insert_lineaged(&conn, "other");
    conn.execute(
        "UPDATE project SET acknowledged_at = NULL WHERE id = ?1",
        [unenrolled.0],
    )
    .unwrap();
    assert_eq!(debt_day_gate(&conn, unenrolled).unwrap(), Gate::Shut);

    assert_eq!(
        debt_day_gate(&conn, ProjectId(9_999)).unwrap(),
        Gate::NotComputable,
        "no subject, no row could be keyed"
    );
}

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
use codotheca_core::index::subject::subject_for_project;
use codotheca_core::index::{open_connection, Index};
use codotheca_core::ledger::marginal::RowDue;
use codotheca_core::ledger::pending::awaiting_restore;
use codotheca_core::ledger::price::{completion_gate, debt_day_gate};
use codotheca_core::ledger::receipt::Receipt;
use codotheca_core::ledger::recompute::{recompute_git_rows, RecomputeCause};
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

/// One record a rebuild left pending on `subject_key`, naming no copy.
fn stage(conn: &rusqlite::Connection, subject_key: &str) {
    conn.execute(
        "INSERT INTO sidecar_pending (source_generation, subject_key, location_keys, record,
                                     queued_at)
         VALUES (1, ?1, '[]', '{}', 1)",
        [subject_key],
    )
    .unwrap();
}

/// The key `project` is exported and matched under.
fn own_key(conn: &rusqlite::Connection, project: ProjectId) -> String {
    subject_for_project(conn, project)
        .unwrap()
        .unwrap()
        .to_key()
}

/// With a restored record for this project still pending and its history not concluded, a closure
/// writes nothing: the record may yet bring its own row back. Once the history job settles, the
/// next closure pays.
#[test]
fn no_retained_write_before_the_match() {
    let (_d, mut conn) = fresh();
    let project = insert_lineaged(&conn, "widget");
    stage(&conn, &own_key(&conn, project));

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

/// `AC-P4-38-27`, §48.8.4: the hold is the project's own. A settled project whose history is
/// queued again is held only by a record the restore would give it — never by a record for a
/// subject that has not come back, nor by one two live projects share and neither is named by.
#[test]
fn ac_p4_38_27_the_restore_hold_is_the_projects_own() {
    let (_d, mut conn) = fresh();
    let project = insert_lineaged(&conn, "widget");
    conn.execute(
        "INSERT INTO location (project_id, kind, distro, path_bytes, path_key, path_display,
                               store_key, presence, repo_kind)
         VALUES (?1, 'linux', '', x'2f77', x'2f77', '/w', 'store', 'present', 'worktree')",
        [project.0],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO project_job_state (project_id, job, state, at) VALUES (?1, 'j4', 'ok', 1)",
        [project.0],
    )
    .unwrap();
    let tx = conn.transaction().unwrap();
    let report = recompute_git_rows(&tx, &[project], RecomputeCause::IdentityConfirm, 2).unwrap();
    tx.commit().unwrap();
    assert_eq!(report.queued, [project], "its history walks again");

    stage(&conn, "lineage:GONE|remote:");
    assert!(
        !awaiting_restore(&conn, project).unwrap(),
        "another subject's record holds nothing"
    );
    assert!(pay(&mut conn, project, 1, NOON_AT_UTC));

    stage(&conn, &own_key(&conn, project));
    assert!(
        awaiting_restore(&conn, project).unwrap(),
        "its own record holds it"
    );
    let tomorrow = LocalInstant {
        unix: NOON + 86_400,
        offset_min: 0,
    };
    assert!(!pay(&mut conn, project, 1, tomorrow));
    assert_eq!(days(&conn).len(), 1);

    // A twin on the same subject: the record names neither copy, so it is neither's and stays
    // pending — and holds neither.
    let twin = insert_lineaged(&conn, "widget twin");
    assert!(!awaiting_restore(&conn, project).unwrap());
    assert!(!awaiting_restore(&conn, twin).unwrap());
    conn.execute("DELETE FROM project WHERE id = ?1", [twin.0])
        .unwrap();
    assert!(awaiting_restore(&conn, project).unwrap());

    conn.execute(
        "UPDATE project_job_state SET state = 'ok' WHERE project_id = ?1 AND job = 'j4'",
        [project.0],
    )
    .unwrap();
    assert!(
        !awaiting_restore(&conn, project).unwrap(),
        "held only until its history concludes"
    );
    assert!(pay(&mut conn, project, 1, tomorrow));

    // A project with no lineage yet is exported under its path, and the settle that fixes its
    // lineage may match a lineage-keyed record no live project holds: until that history
    // concludes, such a record holds it. The settled lineaged project beside it stays free.
    conn.execute(
        "INSERT INTO project (name, seed_basename, acknowledged_at, created_at, updated_at)
         VALUES ('fresh', 'fresh', 1, 1, 1)",
        [],
    )
    .unwrap();
    let fresh_project = ProjectId(conn.last_insert_rowid());
    conn.execute(
        "INSERT INTO location (project_id, kind, distro, path_bytes, path_key, path_display,
                               store_key, presence, repo_kind)
         VALUES (?1, 'linux', '', x'2f6e', x'2f6e', '/n', 'store', 'present', 'worktree')",
        [fresh_project.0],
    )
    .unwrap();
    assert!(own_key(&conn, fresh_project).starts_with("path:"));
    assert!(
        awaiting_restore(&conn, fresh_project).unwrap(),
        "an unclaimed lineage record may be its own"
    );
    assert!(!pay(&mut conn, fresh_project, 1, tomorrow));
    assert!(
        !awaiting_restore(&conn, project).unwrap(),
        "a lineaged project is never held by it"
    );
    conn.execute(
        "INSERT INTO project_job_state (project_id, job, state, at) VALUES (?1, 'j4', 'ok', 1)",
        [fresh_project.0],
    )
    .unwrap();
    assert!(!awaiting_restore(&conn, fresh_project).unwrap());
    assert!(pay(&mut conn, fresh_project, 1, tomorrow));
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
    assert_eq!(debt_day_gate(&conn, project).unwrap(), None);
    assert_eq!(
        completion_gate(&conn, project).unwrap(),
        Some(RowDue::NotComputable),
        "authorship not yet classified"
    );

    conn.execute(
        "UPDATE project SET authored_by_user = 1 WHERE id = ?1",
        [project.0],
    )
    .unwrap();
    commit_days(&conn, project, 0..2);
    assert_eq!(
        completion_gate(&conn, project).unwrap(),
        Some(RowDue::Refused)
    );
    commit_days(&conn, project, 2..3);
    assert_eq!(completion_gate(&conn, project).unwrap(), None);

    conn.execute(
        "UPDATE project SET is_reference = 1 WHERE id = ?1",
        [project.0],
    )
    .unwrap();
    assert_eq!(
        completion_gate(&conn, project).unwrap(),
        Some(RowDue::Refused)
    );
    assert_eq!(
        debt_day_gate(&conn, project).unwrap(),
        Some(RowDue::Refused)
    );

    let unenrolled = insert_lineaged(&conn, "other");
    conn.execute(
        "UPDATE project SET acknowledged_at = NULL WHERE id = ?1",
        [unenrolled.0],
    )
    .unwrap();
    assert_eq!(
        debt_day_gate(&conn, unenrolled).unwrap(),
        Some(RowDue::Refused)
    );

    assert_eq!(
        debt_day_gate(&conn, ProjectId(9_999)).unwrap(),
        Some(RowDue::NotComputable),
        "no subject, no row could be keyed"
    );
}

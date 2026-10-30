//! §38.2 for twins: two live projects can hold one subject (§48.8.4), and every retained key is
//! built from the subject. Each writer finds its row by project first; when its insert meets the
//! key another project's row holds, it acts on that row — so nothing is lost, nothing is paid
//! twice, and the marginal price quotes `0` for it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::clock::{LocalInstant, LocalZone};
use codotheca_core::completion::evaluate::CheckRow;
use codotheca_core::debt::identity::DebtKey;
use codotheca_core::debt::store::{DebtCloseReason, DebtClosure, SweepEffect};
use codotheca_core::debt::xp::pay_debt_day;
use codotheca_core::ledger::focus::credit_segment_close;
use codotheca_core::ledger::latch::latch_completion_checks;
use codotheca_core::ledger::marginal::{marginal_xp, Closure};
use codotheca_core::ledger::verdict::{pay_verdict_award, VerdictAward};
use codotheca_core::protocol::{
    CheckState, ClosedBy, CompletionCheck, DebtScoring, DebtSource, ProjectId,
};
use codotheca_core::session::measure::focus_seconds;
use codotheca_core::session::segment::ClosedSegment;
use codotheca_core::session::store::{self, SessionOpen};
use codotheca_core::testing::zone::FixedZone;
use codotheca_core::testing::TempIndex;

/// 2026-09-17 12:00:00 UTC.
const NOON: i64 = 1_789_646_400;
const NOON_AT_UTC: LocalInstant = LocalInstant {
    unix: NOON,
    offset_min: 0,
};

/// An enrolled, authored project at `path`, past the latch's commit-day gate; `lineage` gives it
/// that lineage, `None` leaves its subject the path.
fn project(index: &TempIndex, path: &str, lineage: Option<&str>) -> ProjectId {
    let project = index.insert_project();
    index.insert_location(project, path);
    index.set_project_i64(project, "acknowledged_at", 1);
    index.set_project_i64(project, "authored_by_user", 1);
    let conn = index.index().conn();
    conn.execute(
        "UPDATE project SET lineage_key = ?2 WHERE id = ?1",
        rusqlite::params![project.0, lineage],
    )
    .unwrap();
    for day in 0..3 {
        conn.execute(
            "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                    track, meta)
             VALUES (?1, 0, ?2, 'git', 'commit_day', ?3, 'git', '{}')",
            rusqlite::params![
                day * 86_400,
                project.0,
                format!("commit_day:{}:{day}", project.0)
            ],
        )
        .unwrap();
    }
    project
}

/// One fixed, scored item of `source`.
fn fixed(source: DebtSource) -> SweepEffect {
    let mut key = DebtKey::content("subject", &format!("{:064x}", 7), 0);
    key.source = source;
    SweepEffect {
        closed: vec![DebtClosure {
            key,
            reason: DebtCloseReason::Fixed,
            scoring: DebtScoring::Scored,
        }],
        ..SweepEffect::default()
    }
}

fn pay_day(
    index: &mut TempIndex,
    project: ProjectId,
    source: DebtSource,
    at: LocalInstant,
) -> bool {
    let tx = index.index_mut().conn_mut().transaction().unwrap();
    let payout = pay_debt_day(&tx, project, &fixed(source), at).unwrap();
    tx.commit().unwrap();
    payout.wrote_row
}

/// A counted segment of `project` from `start`, `seconds` long, closed and credited.
fn focus(index: &mut TempIndex, project: ProjectId, start: i64, seconds: i64) {
    let location: i64 = index
        .index()
        .conn()
        .query_row(
            "SELECT id FROM location WHERE project_id = ?1",
            [project.0],
            |r| r.get(0),
        )
        .unwrap();
    let tx = index.index_mut().conn_mut().transaction().unwrap();
    let session = store::open_session(
        &tx,
        &SessionOpen {
            project_id: project,
            location_id: Some(codotheca_core::protocol::LocationId(location)),
            target_id: None,
            started_at: start,
        },
    )
    .unwrap();
    let segment = store::open_segment(&tx, session, start, true).unwrap();
    store::close_segment(
        &tx,
        segment,
        &ClosedSegment {
            started_at: start,
            ended_at: start + seconds,
            credited_seconds: seconds,
            closed_by: ClosedBy::Idle,
        },
    )
    .unwrap();
    credit_segment_close(&tx, &FixedZone(0), segment).unwrap();
    tx.commit().unwrap();
}

fn latch(index: &mut TempIndex, project: ProjectId, check: CompletionCheck) -> u32 {
    let tx = index.index_mut().conn_mut().transaction().unwrap();
    let rows = [CheckRow {
        key: check,
        state: CheckState::Pass,
        user_na: None,
        unknown_reason: None,
        observed_at: NOON,
    }];
    let written = latch_completion_checks(&tx, project, &rows, NOON_AT_UTC).unwrap();
    tx.commit().unwrap();
    written
}

/// `(project_id, meta)` of every row of `kind`, by id.
fn rows(index: &TempIndex, kind: &str) -> Vec<(i64, serde_json::Value)> {
    index
        .index()
        .conn()
        .prepare("SELECT project_id, meta FROM xp_events WHERE kind = ?1 ORDER BY id")
        .unwrap()
        .query_map([kind], |r| {
            Ok((
                r.get(0)?,
                serde_json::from_str(&r.get::<_, String>(1)?).unwrap(),
            ))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn quote(index: &TempIndex, project: ProjectId, closure: Closure) -> Option<u32> {
    marginal_xp(index.index().conn(), project, &closure, NOON_AT_UTC).unwrap()
}

/// `AC-P4-38-28`, §38.2: two live projects on one subject lose nothing and are paid once, and a
/// project whose subject moved from its path to a lineage is still paid exactly once.
#[test]
fn ac_p4_38_28_twins_lose_nothing_and_are_paid_once() {
    let mut index = TempIndex::new();
    let holder = project(&index, "/w/a", Some("abc123"));
    let twin = project(&index, "/w/b", Some("abc123"));

    // Focus: the twin's seconds land on the holder's row, so the date's rows sum to the union.
    focus(&mut index, holder, NOON, 600);
    focus(&mut index, twin, NOON + 1_000, 600);
    let day = FixedZone(0).at(NOON).day();
    let union = focus_seconds(index.index().conn(), &FixedZone(0), day, NOON + 86_400).unwrap();
    let focus_rows = rows(&index, "focus");
    let stored: i64 = focus_rows
        .iter()
        .map(|(_, meta)| meta["seconds"].as_i64().unwrap())
        .sum();
    eprintln!("focus on the twins' date: stored {stored}, union {union}");
    assert_eq!(union, 1_200);
    assert_eq!(stored, union, "{focus_rows:?}");
    assert_eq!(focus_rows.len(), 1, "one row per subject and date");

    // `debt_day`: the twin's sources merge into the holder's day.
    let debt = Closure {
        debt_day: Some(DebtScoring::Scored),
        lights: None,
        verdict: false,
    };
    assert!(pay_day(
        &mut index,
        holder,
        DebtSource::TodoMarker,
        NOON_AT_UTC
    ));
    assert_eq!(
        quote(&index, twin, debt),
        Some(0),
        "the holder's day is paid"
    );
    assert!(!pay_day(
        &mut index,
        twin,
        DebtSource::MissingReadme,
        NOON_AT_UTC
    ));
    let days = rows(&index, "debt_day");
    assert_eq!(days.len(), 1, "{days:?}");
    assert_eq!(days[0].0, holder.0);
    assert_eq!(
        days[0].1["sources"],
        serde_json::json!(["missing_readme", "todo_marker"])
    );
    assert_eq!(days[0].1["closed"], 2);

    // The latch: the twin's quote is 0 and it writes nothing.
    let lights = Closure {
        debt_day: None,
        lights: Some(CompletionCheck::Readme),
        verdict: false,
    };
    assert_ne!(quote(&index, holder, lights), Some(0));
    assert_eq!(latch(&mut index, holder, CompletionCheck::Readme), 1);
    assert_eq!(quote(&index, twin, lights), Some(0));
    assert_eq!(latch(&mut index, twin, CompletionCheck::Readme), 0);
    assert_eq!(rows(&index, "completion_check").len(), 1);

    // The verdict: paid once for the subject.
    let tx = index.index_mut().conn_mut().transaction().unwrap();
    assert!(matches!(
        pay_verdict_award(&tx, holder, "keep", NOON_AT_UTC).unwrap(),
        VerdictAward::Paid(_)
    ));
    assert_eq!(
        pay_verdict_award(&tx, twin, "keep", NOON_AT_UTC).unwrap(),
        VerdictAward::AlreadyPaid
    );
    tx.commit().unwrap();

    // A project paid under its path, then given a lineage: its day is found by project, and the
    // same date pays nothing more.
    let moved = project(&index, "/w/m", None);
    assert!(pay_day(
        &mut index,
        moved,
        DebtSource::TodoMarker,
        NOON_AT_UTC
    ));
    index
        .index()
        .conn()
        .execute(
            "UPDATE project SET lineage_key = 'def456' WHERE id = ?1",
            [moved.0],
        )
        .unwrap();
    assert_eq!(quote(&index, moved, debt), Some(0));
    assert!(!pay_day(&mut index, moved, DebtSource::CiRed, NOON_AT_UTC));
    let moved_days: Vec<_> = rows(&index, "debt_day")
        .into_iter()
        .filter(|(p, _)| *p == moved.0)
        .collect();
    assert_eq!(moved_days.len(), 1, "{moved_days:?}");
    assert_eq!(moved_days[0].1["closed"], 2);
}

//! §40.2–§40.3's populations and gates, each driven through the J4 settle that writes the
//! commit-days they read.
//!
//! Every test prints the rows it read, so a fixture that wrote no commit-day cannot pass by
//! awarding nothing over nothing.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use codotheca_core::badges::evaluate::{evaluate, sweep_unlatched, EvalCtx};
use codotheca_core::badges::{
    COMEBACK_GAP_DAYS, CURATOR_PROJECTS, FIREFIGHTER_PAIRS, MARATHON_CREDITED_SECONDS,
};
use codotheca_core::clock::{local_date, LocalInstant};
use codotheca_core::debt::identity::DebtKey;
use codotheca_core::debt::store::{DebtCloseReason, DebtClosure, SweepEffect};
use codotheca_core::debt::xp::pay_debt_day;
use codotheca_core::index::Index;
use codotheca_core::ledger::price::REVIVAL_GAP_DAYS;
use codotheca_core::protocol::{BadgeKind, CloseReason, ClosedBy, ProjectId, SessionId};
use codotheca_core::protocol::{DebtScoring, DebtSource};
use codotheca_core::session::orphan;
use codotheca_core::session::segment::ClosedSegment;
use codotheca_core::session::store;
use codotheca_core::testing::zone::FixedZone;
use support::badges_rig::{
    award_row, connect_ci, opening, orphaned_session, plain_project, plant_commit_day,
    plant_repairs, red_then_green, repair_pairs, session_index, session_row, AwardRow, BadgesRig,
    Flags, SessionLib, T0,
};

/// A local day number in early 2024: the first commit-day of every gap below.
const D1: i64 = 19_724;

/// `project`'s commit-day rows.
fn commit_days(rig: &BadgesRig, project: i64) -> i64 {
    let guard = rig.index.lock().unwrap();
    let n = guard
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM xp_events WHERE kind = 'commit_day' AND project_id = ?1",
            [project],
            |r| r.get(0),
        )
        .unwrap();
    drop(guard);
    n
}

/// With `gap = d2 − d1`, Necromancer is not awarded one below §38's dormancy constant and is
/// awarded at it — the constant read from §38's module.
#[test]
fn ac_p4_40_10_necromancer_is_the_gap_rule_at_the_constant() {
    let mut rig = BadgesRig::new();
    let below = rig.project("below", &[D1, D1 + REVIVAL_GAP_DAYS - 1], Flags::default());
    rig.settle_j4(below);
    eprintln!(
        "gap {}: {} commit-days, award {:?}",
        REVIVAL_GAP_DAYS - 1,
        commit_days(&rig, below.0),
        rig.award("necromancer")
    );
    assert_eq!(commit_days(&rig, below.0), 2);
    assert_eq!(rig.award("necromancer"), None);

    let at = rig.project("at", &[D1, D1 + REVIVAL_GAP_DAYS], Flags::default());
    rig.settle_j4(at);
    let award = rig.award("necromancer");
    eprintln!("gap {REVIVAL_GAP_DAYS}: award {award:?}");
    let (earned_on, project, evidence) = award.expect("no award at the constant");
    assert_eq!(earned_on, local_date(D1 + REVIVAL_GAP_DAYS));
    assert_eq!(project, Some(at.0));
    assert_eq!(evidence["gapDays"], REVIVAL_GAP_DAYS);
}

/// §40.2's **H**: a qualifying gap in a shallow project, and the same gap in a Reference
/// project, yield no Necromancer.
#[test]
fn ac_p4_40_9_shallow_and_reference_yield_no_necromancer() {
    let mut rig = BadgesRig::new();
    let gap = [D1, D1 + REVIVAL_GAP_DAYS];
    let shallow = rig.project(
        "shallow",
        &gap,
        Flags {
            shallow: true,
            reference: false,
        },
    );
    rig.settle_j4(shallow);
    // J4 still writes a shallow project's days; only the population keeps them out.
    eprintln!(
        "shallow: {} commit-days, award {:?}",
        commit_days(&rig, shallow.0),
        rig.award("necromancer")
    );
    assert_eq!(commit_days(&rig, shallow.0), 2);
    assert_eq!(rig.award("necromancer"), None);

    let reference = rig.project(
        "reference",
        &gap,
        Flags {
            shallow: false,
            reference: true,
        },
    );
    rig.settle_j4(reference);
    eprintln!(
        "reference: {} commit-days, award {:?}",
        commit_days(&rig, reference.0),
        rig.award("necromancer")
    );
    assert_eq!(rig.award("necromancer"), None);
}

/// §40.3: Comeback is evaluated only while history is complete. The covered project's gap
/// qualifies, but a present-location project has not been walked: no row. Its J4 settles, and
/// that settle awards — its own days close nothing.
#[test]
fn ac_p4_40_9_comeback_waits_for_complete_history() {
    let mut rig = BadgesRig::new();
    let covered = rig.project("covered", &[D1, D1 + COMEBACK_GAP_DAYS], Flags::default());
    let unwalked = rig.project("unwalked", &[D1 + 100], Flags::default());
    rig.settle_j4(covered);
    let before = (rig.history_complete(), rig.award("comeback"));
    eprintln!(
        "covered {} commit-days; historyComplete {}, award {:?}",
        commit_days(&rig, covered.0),
        before.0,
        before.1
    );
    assert_eq!(commit_days(&rig, covered.0), 2);
    assert!(!before.0, "history read complete with a project unwalked");
    assert_eq!(before.1, None, "comeback awarded over incomplete history");

    rig.settle_j4(unwalked);
    let award = rig.award("comeback");
    eprintln!(
        "historyComplete {}, award {award:?}",
        rig.history_complete()
    );
    let (earned_on, _, evidence) = award.expect("no comeback once history completed");
    assert_eq!(earned_on, local_date(D1 + COMEBACK_GAP_DAYS));
    assert_eq!(evidence["gapDays"], COMEBACK_GAP_DAYS);
}

/// `project`'s `primary_language`, as a finished J3 would have written it.
fn set_language(rig: &BadgesRig, project: ProjectId, language: Option<&str>) {
    let guard = rig.index.lock().unwrap();
    guard
        .conn()
        .execute(
            "UPDATE project SET primary_language = ?2 WHERE id = ?1",
            rusqlite::params![project.0, language],
        )
        .unwrap();
    drop(guard);
}

/// §40.2's **H** and §40.3's NULL rule: four counted languages, and a fifth only from a shallow
/// project, a Reference project, or a project whose language is not computed — no Polyglot at
/// any step. A fifth counted language then awards, so the library was one language short.
#[test]
fn ac_p4_40_9_shallow_reference_and_null_languages_make_no_polyglot() {
    let mut rig = BadgesRig::new();
    for (name, day, language) in [
        ("go", D1 + 1, "Go"),
        ("python", D1 + 2, "Python"),
        ("ruby", D1 + 3, "Ruby"),
        ("c", D1 + 4, "C"),
    ] {
        let project = rig.project(name, &[day], Flags::default());
        set_language(&rig, project, Some(language));
        rig.settle_j4(project);
    }

    let shallow = rig.project(
        "shallow",
        &[D1 + 5],
        Flags {
            shallow: true,
            reference: false,
        },
    );
    set_language(&rig, shallow, Some("Rust"));
    rig.settle_j4(shallow);
    eprintln!(
        "shallow Rust: {} commit-days, award {:?}",
        commit_days(&rig, shallow.0),
        rig.award("polyglot")
    );
    assert_eq!(commit_days(&rig, shallow.0), 1);
    assert_eq!(
        rig.award("polyglot"),
        None,
        "a shallow project's language counted"
    );

    let reference = rig.project(
        "reference",
        &[D1 + 6],
        Flags {
            shallow: false,
            reference: true,
        },
    );
    set_language(&rig, reference, Some("Zig"));
    rig.settle_j4(reference);
    eprintln!(
        "reference Zig: {} commit-days, award {:?}",
        commit_days(&rig, reference.0),
        rig.award("polyglot")
    );
    assert_eq!(
        rig.award("polyglot"),
        None,
        "a Reference project's language counted"
    );

    let unknown = rig.project("unknown", &[D1 + 7], Flags::default());
    set_language(&rig, unknown, None);
    rig.settle_j4(unknown);
    eprintln!(
        "NULL language: {} commit-days, award {:?}",
        commit_days(&rig, unknown.0),
        rig.award("polyglot")
    );
    assert_eq!(commit_days(&rig, unknown.0), 1);
    assert_eq!(rig.award("polyglot"), None, "a NULL language counted");

    set_language(&rig, unknown, Some("Rust"));
    rig.settle_j4(unknown);
    let award = rig.award("polyglot");
    eprintln!("the unknown project computed as Rust: award {award:?}");
    let (earned_on, _, evidence) = award.expect("five counted languages awarded nothing");
    assert_eq!(earned_on, local_date(D1 + 7));
    assert_eq!(
        evidence["languages"],
        serde_json::json!(["Go", "Python", "Ruby", "C", "Rust"])
    );
}

/// §40.4: Comeback at `gap = COMEBACK_GAP_DAYS − 1` is not awarded and at the constant is.
#[test]
fn ac_p4_40_10_comeback_is_the_gap_rule_at_its_constant() {
    let mut below = BadgesRig::new();
    let short = below.project("below", &[D1, D1 + COMEBACK_GAP_DAYS - 1], Flags::default());
    below.settle_j4(short);
    eprintln!(
        "gap {}: {} commit-days, historyComplete {}, award {:?}",
        COMEBACK_GAP_DAYS - 1,
        commit_days(&below, short.0),
        below.history_complete(),
        below.award("comeback")
    );
    assert_eq!(commit_days(&below, short.0), 2);
    assert!(below.history_complete());
    assert_eq!(below.award("comeback"), None);

    let mut at = BadgesRig::new();
    let enough = at.project("at", &[D1, D1 + COMEBACK_GAP_DAYS], Flags::default());
    at.settle_j4(enough);
    let award = at.award("comeback");
    eprintln!("gap {COMEBACK_GAP_DAYS}: award {award:?}");
    let (earned_on, _, evidence) = award.expect("no comeback at the constant");
    assert_eq!(earned_on, local_date(D1 + COMEBACK_GAP_DAYS));
    assert_eq!(evidence["gapDays"], COMEBACK_GAP_DAYS);
}

/// A Reference verdict must not open a Comeback gap. A project's commit-day closes the gap
/// between two others; the project is then ruled Reference and its commit-days are deleted, as
/// the reconcile does. Its recorded user span still lies across the gap, so the gap is withheld.
#[test]
fn a_reference_verdict_cannot_open_a_comeback_gap() {
    let mut rig = BadgesRig::new();
    let mine = rig.project("mine", &[D1, D1 + 40], Flags::default());
    let theirs = rig.project("theirs", &[D1 + 20], Flags::default());
    rig.settle_j4(mine);
    rig.settle_j4(theirs);
    assert_eq!(rig.award("comeback"), None, "a 20-day gap awarded comeback");

    rig.sql(&format!(
        "UPDATE project SET is_reference = 1 WHERE id = {0};
         DELETE FROM xp_events WHERE kind = 'commit_day' AND project_id = {0};",
        theirs.0
    ));
    rig.settle_j4(mine);
    let span: (Option<i64>, Option<i64>) = {
        let guard = rig.index.lock().unwrap();
        let span = guard
            .conn()
            .query_row(
                "SELECT first_commit_at, last_user_commit_at FROM project WHERE id = ?1",
                [theirs.0],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        drop(guard);
        span
    };
    let award = rig.award("comeback");
    eprintln!(
        "theirs: {} commit-days, span {span:?}; mine: {} commit-days; historyComplete {}, \
         award {award:?}",
        commit_days(&rig, theirs.0),
        commit_days(&rig, mine.0),
        rig.history_complete()
    );
    assert_eq!(commit_days(&rig, theirs.0), 0);
    assert_eq!(commit_days(&rig, mine.0), 2);
    assert!(span.1.is_some(), "J4 recorded no user span");
    assert!(rig.history_complete());
    assert_eq!(
        award, None,
        "a Reference verdict's deleted days opened a gap"
    );
}

/// The session tests' first instant, 2023-11-14 22:13:20 UTC.
const T_SESSION: i64 = 1_700_000_000;

/// Three days after it: when the next process starts.
const LATER: i64 = T_SESSION + 3 * 86_400;

/// A session a previous process left open at `started_at` whose one segment was already closed
/// over `[started_at, started_at + wall]` with `credited` seconds of credit.
fn orphan_with_closed_segment(
    lib: &mut SessionLib,
    started_at: i64,
    wall: i64,
    credited: i64,
) -> SessionId {
    let open = opening(lib, started_at);
    let tx = lib.index.index_mut().conn_mut().transaction().unwrap();
    let session = store::open_session(&tx, &open).unwrap();
    let segment = store::open_segment(&tx, session, started_at, false).unwrap();
    store::close_segment(
        &tx,
        segment,
        &ClosedSegment {
            started_at,
            ended_at: started_at + wall,
            credited_seconds: credited,
            closed_by: ClosedBy::Idle,
        },
    )
    .unwrap();
    tx.commit().unwrap();
    session
}

/// §40.3: Marathon reads credited seconds, never wall-clock. A session spanning
/// `MARATHON_CREDITED_SECONDS` of wall-clock with one second less credit is not awarded; one
/// credited exactly the constant is.
#[test]
fn ac_p4_40_14_credit_not_wall_clock() {
    let mut lib = session_index();
    let short = orphan_with_closed_segment(
        &mut lib,
        T_SESSION,
        MARATHON_CREDITED_SECONDS,
        MARATHON_CREDITED_SECONDS - 1,
    );
    orphan::close_orphans(lib.index.index_mut(), LATER, &FixedZone(0)).unwrap();
    let (ended_at, credited) = session_row(&lib.index, short);
    let none = award_row(lib.index.index().conn(), "marathon");
    eprintln!(
        "wall {:?}, credited {credited}: {none:?}",
        ended_at.map(|e| e - T_SESSION)
    );
    assert_eq!(ended_at, Some(T_SESSION + MARATHON_CREDITED_SECONDS));
    assert_eq!(credited, MARATHON_CREDITED_SECONDS - 1);
    assert_eq!(none, None, "wall-clock awarded marathon");

    let start = T_SESSION + 86_400;
    orphan_with_closed_segment(
        &mut lib,
        start,
        MARATHON_CREDITED_SECONDS,
        MARATHON_CREDITED_SECONDS,
    );
    orphan::close_orphans(lib.index.index_mut(), LATER, &FixedZone(0)).unwrap();
    let award = award_row(lib.index.index().conn(), "marathon");
    eprintln!("credited the constant: {award:?}");
    let award = award.expect("no marathon at the constant");
    assert_eq!(award.earned_at, start + MARATHON_CREDITED_SECONDS);
    assert_eq!(award.evidence["creditedSeconds"], MARATHON_CREDITED_SECONDS);
}

/// §40.5's second close path: `close_orphans` closes a crashed session at its watermark, and
/// that close awards, dated by the zone at the session's own end.
#[test]
fn ac_p4_40_14_a_session_closed_by_close_orphans_awards() {
    let mut lib = session_index();
    orphaned_session(&mut lib, T_SESSION, MARATHON_CREDITED_SECONDS);
    let report = orphan::close_orphans(lib.index.index_mut(), LATER, &FixedZone(-300)).unwrap();
    let award = award_row(lib.index.index().conn(), "marathon");
    eprintln!("{report:?}: {award:?}");
    assert_eq!(report.sessions_closed, 1);
    let award = award.expect("close_orphans wrote no marathon");
    assert_eq!(award.earned_at, T_SESSION + MARATHON_CREDITED_SECONDS);
    assert_eq!(award.tz_offset_min, -300);
    assert_eq!(award.earned_on, "2023-11-14");
    assert_eq!(award.project_id, Some(lib.project.0));
    assert_eq!(
        award.subject_key.as_deref(),
        Some("lineage:marathon|remote:")
    );
    assert_eq!(award.evidence["projectName"], "marathon");
}

/// A session the app recorded before badges existed is awarded when the evaluator next runs
/// over the stored rows — the sweep the day close and the arming run — dated by its own
/// `ended_at`, never by the sweep.
#[test]
fn a_session_recorded_before_the_migration_is_dated_by_its_own_ended_at() {
    let mut lib = session_index();
    let end = T_SESSION + MARATHON_CREDITED_SECONDS;
    let session = orphan_with_closed_segment(
        &mut lib,
        T_SESSION,
        MARATHON_CREDITED_SECONDS,
        MARATHON_CREDITED_SECONDS,
    );
    let tx = lib.index.index_mut().conn_mut().transaction().unwrap();
    store::close_session(&tx, session, end, CloseReason::Stop, end).unwrap();
    tx.commit().unwrap();
    let stored = award_row(lib.index.index().conn(), "marathon");
    assert_eq!(stored, None, "the store's close is not an award site");

    let sweep = lib.index.index_mut().conn_mut().transaction().unwrap();
    let wrote = sweep_unlatched(
        &sweep,
        &EvalCtx {
            now: LATER,
            zone: &FixedZone(-300),
        },
    )
    .unwrap();
    sweep.commit().unwrap();
    let award = award_row(lib.index.index().conn(), "marathon");
    eprintln!("wrote {wrote:?}: {award:?}");
    let award = award.expect("the stored session was not awarded");
    assert_eq!(award.earned_at, end);
    assert_eq!(award.earned_on, "2023-11-14");
    assert_eq!(award.awarded_on, "2023-11-17");
}

/// `curator` evaluated over `rig`'s library at `T0`, and its row after.
fn curator_over(rig: &BadgesRig) -> Option<AwardRow> {
    let mut guard = rig.index.lock().unwrap();
    let wrote = guard
        .with_tx(|tx| {
            evaluate(
                tx,
                &[BadgeKind::Curator],
                &EvalCtx {
                    now: T0,
                    zone: &FixedZone(0),
                },
            )
        })
        .unwrap();
    let award = award_row(guard.conn(), "curator");
    drop(guard);
    eprintln!("wrote {wrote:?}: {award:?}");
    award
}

/// Notched gold is gold: ten projects whose every evaluable check is lit, each with fewer than
/// ten evaluable, award Curator, and the evidence names each with its evaluable count.
#[test]
fn ac_p4_40_13_notched_gold_counts_with_its_evaluable_count() {
    let mut rig = BadgesRig::new();
    let scored: Vec<(ProjectId, i64)> = (0..CURATOR_PROJECTS)
        .map(|i| {
            let project = rig.project(&format!("p{i}"), &[D1], Flags::default());
            (project, 6 + i64::try_from(i % 4).unwrap())
        })
        .collect();
    rig.at_top_rung(&scored);
    let award = curator_over(&rig).expect("ten notched-gold projects wrote no curator");
    assert_eq!(award.earned_at, T0);
    assert_eq!(award.awarded_at, T0);
    assert_eq!(award.project_id, None);
    assert_eq!(award.subject_key, None);
    let listed: Vec<(String, i64)> = award.evidence["projects"]
        .as_array()
        .expect("curator evidence lists its projects")
        .iter()
        .map(|p| {
            (
                p["name"].as_str().unwrap().to_owned(),
                p["evaluable"].as_i64().unwrap(),
            )
        })
        .collect();
    let want: Vec<(String, i64)> = scored
        .iter()
        .enumerate()
        .map(|(i, &(_, evaluable))| (format!("p{i}"), evaluable))
        .collect();
    assert_eq!(listed, want);
}

/// **P**'s present-location clause: nine present projects at gold and a tenth whose every copy
/// is removed make nine, which is not ten.
#[test]
fn ac_p4_40_13_a_project_with_no_present_location_is_not_counted() {
    let mut rig = BadgesRig::new();
    let scored: Vec<(ProjectId, i64)> = (0..CURATOR_PROJECTS)
        .map(|i| (rig.project(&format!("p{i}"), &[D1], Flags::default()), 10))
        .collect();
    rig.at_top_rung(&scored);
    let gone = scored[CURATOR_PROJECTS - 1].0;
    rig.sql(&format!(
        "UPDATE location SET removed_at = {T0} WHERE project_id = {}",
        gone.0
    ));
    assert_eq!(curator_over(&rig), None);
}

/// Seconds in a day.
const DAY: i64 = 86_400;

/// An hour, in seconds.
const HOUR: i64 = 3_600;

/// Firefighter evaluated over `index` at `T0`, and the kinds it wrote.
fn firefighter_over(index: &mut Index) -> Vec<BadgeKind> {
    index
        .with_tx(|tx| {
            evaluate(
                tx,
                &[BadgeKind::Firefighter],
                &EvalCtx {
                    now: T0,
                    zone: &FixedZone(0),
                },
            )
        })
        .unwrap()
}

/// One case's count, printed, with the award it left (none, for every exclusion).
fn counted(index: &mut Index, case: &str) -> Option<AwardRow> {
    let wrote = firefighter_over(index);
    let pairs = repair_pairs(index);
    let award = award_row(index.conn(), "firefighter");
    eprintln!(
        "{case}: {} qualifying pair(s), wrote {wrote:?}, award {award:?}",
        pairs.len()
    );
    award
}

/// `project`'s `debt_day` `same_day` list on local day `day`, when the day holds a row.
fn same_day_of(index: &Index, project: ProjectId, day: i64) -> Option<serde_json::Value> {
    index
        .conn()
        .query_row(
            "SELECT meta FROM xp_events
              WHERE kind = 'debt_day' AND project_id = ?1 AND ts / 86400 = ?2",
            rusqlite::params![project.0, day],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .map(|meta| serde_json::from_str::<serde_json::Value>(&meta).unwrap()["same_day"].clone())
}

/// §40.3, §40.10: Firefighter counts only an attributed same-day repair. Over a library already
/// holding one pair fewer than `FIREFIGHTER_PAIRS`, an invalidated `ci_red` closure, a fixed one
/// of an item first seen the local day before, a same-day repair on a date with no commit-day,
/// and a same-day repair on an unenrolled project each add no pair — any one of them counted
/// would award. The tenth qualifying pair then awards, naming that pair's project and date.
///
/// The invalidated closure is driven at `pay_debt_day`'s input: the singleton settle closes
/// nothing as invalidated — only the advisory feed's withdrawal does.
#[test]
fn ac_p4_40_11() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open_at(dir.path(), T0).unwrap();
    let base: Vec<i64> = (0..FIREFIGHTER_PAIRS - 1)
        .map(|i| D1 + i64::try_from(i).unwrap())
        .collect();
    plant_repairs(&mut index, "base", &base);
    assert_eq!(counted(&mut index, "the planted library"), None);
    assert_eq!(repair_pairs(&index).len(), FIREFIGHTER_PAIRS - 1);

    an_invalidated_repair(&mut index);
    a_repair_first_seen_the_day_before(&mut index);
    a_repair_with_no_commit_day(&mut index);
    an_unenrolled_repair(&mut index);
    the_tenth_pair(&mut index);
}

/// An invalidated `ci_red` closure beside a paying one, on a date the project committed, driven
/// at `pay_debt_day`'s input.
fn an_invalidated_repair(index: &mut Index) {
    let withdrawn = plain_project(index, "withdrawn", true);
    let day = D1 + 20;
    plant_commit_day(index, withdrawn, day);
    let closure = |source, reason| DebtClosure {
        key: DebtKey::singleton("lineage:withdrawn|remote:", source),
        reason,
        scoring: DebtScoring::Scored,
        first_seen_day: day,
    };
    let effect = SweepEffect {
        closed: vec![
            closure(DebtSource::CiRed, DebtCloseReason::Invalidated),
            closure(DebtSource::TodoMarker, DebtCloseReason::Fixed),
        ],
        ..SweepEffect::default()
    };
    index
        .with_tx(|tx| {
            pay_debt_day(
                tx,
                withdrawn,
                &effect,
                LocalInstant {
                    unix: day * DAY + 12 * HOUR,
                    offset_min: 0,
                },
            )
            .unwrap();
            Ok(())
        })
        .unwrap();
    assert_eq!(
        same_day_of(index, withdrawn, day),
        Some(serde_json::json!(["todo_marker"])),
        "the day's same_day lists the invalidated ci_red closure"
    );
    assert_eq!(counted(index, "an invalidated ci_red closure"), None);
}

/// Red the evening before, green the next morning: a repair, not a same-day one.
fn a_repair_first_seen_the_day_before(index: &mut Index) {
    let earlier = plain_project(index, "earlier", true);
    connect_ci(index, earlier);
    let day = D1 + 21;
    plant_commit_day(index, earlier, day);
    plant_commit_day(index, earlier, day + 1);
    red_then_green(
        index,
        earlier,
        day * DAY + 22 * HOUR,
        (day + 1) * DAY + 9 * HOUR,
    );
    assert_eq!(
        same_day_of(index, earlier, day + 1),
        Some(serde_json::json!([])),
        "the repair did not pay, or counted as same-day"
    );
    assert_eq!(counted(index, "an item first seen the day before"), None);
}

/// A same-day repair on a date the project has no commit-day.
fn a_repair_with_no_commit_day(index: &mut Index) {
    let uncommitted = plain_project(index, "uncommitted", true);
    connect_ci(index, uncommitted);
    let day = D1 + 23;
    plant_commit_day(index, uncommitted, day - 1);
    red_then_green(
        index,
        uncommitted,
        day * DAY + 9 * HOUR,
        day * DAY + 15 * HOUR,
    );
    assert_eq!(
        same_day_of(index, uncommitted, day),
        Some(serde_json::json!(["ci_red"])),
        "the same-day repair was not recorded"
    );
    assert_eq!(counted(index, "a same-day repair with no commit-day"), None);
}

/// A same-day repair on a date it committed, on a project never acknowledged.
fn an_unenrolled_repair(index: &mut Index) {
    let unenrolled = plain_project(index, "unenrolled", false);
    connect_ci(index, unenrolled);
    index
        .conn()
        .execute(
            "UPDATE project SET acknowledged_at = NULL WHERE id = ?1",
            [unenrolled.0],
        )
        .unwrap();
    let day = D1 + 24;
    plant_commit_day(index, unenrolled, day);
    red_then_green(
        index,
        unenrolled,
        day * DAY + 9 * HOUR,
        day * DAY + 15 * HOUR,
    );
    assert_eq!(
        same_day_of(index, unenrolled, day),
        None,
        "an unenrolled project was paid"
    );
    assert_eq!(counted(index, "an unenrolled project's repair"), None);
}

/// The tenth qualifying pair: the award names its project and date.
fn the_tenth_pair(index: &mut Index) {
    let tenth = plain_project(index, "tenth", true);
    connect_ci(index, tenth);
    let day = D1 + 25;
    plant_commit_day(index, tenth, day);
    red_then_green(index, tenth, day * DAY + 9 * HOUR, day * DAY + 15 * HOUR);
    let award = counted(index, "the tenth qualifying pair").expect("ten pairs awarded nothing");
    assert_eq!(repair_pairs(index).len(), FIREFIGHTER_PAIRS);
    assert_eq!(award.earned_on, local_date(day));
    assert_eq!(award.earned_at, day * DAY);
    assert_eq!(award.project_id, None);
    assert_eq!(
        award.evidence,
        serde_json::json!({ "projectName": "tenth" })
    );
}

/// `undertaker`'s row, when it is held.
fn undertaker_over(rig: &BadgesRig) -> Option<AwardRow> {
    let guard = rig.index.lock().unwrap();
    let row = award_row(guard.conn(), "undertaker");
    drop(guard);
    row
}

/// §40.3: Undertaker counts distinct projects, never rows. One project archived, unarchived and
/// archived again carries one row; a second row moved onto it — a merge reparents the absorbed
/// project's `xp_events` onto the survivor — still counts it once. Five rows over four projects
/// award nothing; the fifth distinct project does, and the evidence lists the first five.
#[test]
fn ac_p4_40_15_one_project_archived_twice_counts_once() {
    let mut rig = BadgesRig::new();
    let mut at = T0;
    let mut observe = |lib: &BadgesRig, project: ProjectId| {
        at += 3_600;
        lib.observe_archive(project, at, 0);
    };
    let twice = rig.archivable("twice");
    rig.set_archived(twice, true);
    observe(&rig, twice);
    rig.set_archived(twice, false);
    observe(&rig, twice);
    rig.set_archived(twice, true);
    observe(&rig, twice);
    assert_eq!(
        rig.clean_archive_rows(twice).len(),
        1,
        "re-archiving wrote a second row"
    );

    let absorbed = rig.archivable("absorbed");
    rig.set_archived(absorbed, true);
    observe(&rig, absorbed);
    assert_eq!(rig.clean_archive_rows(absorbed).len(), 1);
    rig.sql(&format!(
        "UPDATE xp_events SET project_id = {survivor} WHERE project_id = {gone};
         UPDATE project SET merged_into = {survivor} WHERE id = {gone};",
        survivor = twice.0,
        gone = absorbed.0
    ));
    assert_eq!(rig.clean_archive_rows(twice).len(), 2);

    for name in ["second", "third", "fourth"] {
        let p = rig.archivable(name);
        rig.set_archived(p, true);
        observe(&rig, p);
        assert_eq!(rig.clean_archive_rows(p).len(), 1, "{name} was not paid");
    }
    let rows = rig.xp_event_count_of("clean_archive");
    let held = undertaker_over(&rig);
    eprintln!("{rows} clean_archive rows over four projects: {held:?}");
    assert_eq!(rows, 5);
    assert_eq!(held, None, "five rows over four projects awarded");

    let fifth = rig.archivable("fifth");
    rig.set_archived(fifth, true);
    observe(&rig, fifth);
    let award = undertaker_over(&rig).expect("five distinct projects awarded nothing");
    eprintln!("after the fifth distinct project: {award:?}");
    assert_eq!(award.earned_at, rig.clean_archive_rows(fifth)[0].0);
    let listed: Vec<&str> = award.evidence["projects"]
        .as_array()
        .expect("undertaker evidence lists its projects")
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(listed, ["twice", "second", "third", "fourth", "fifth"]);
}

/// §40.3: one gap arithmetic. The dormancy constant's decimal value, read from §38's constant,
/// is spelled as no integer literal anywhere under `core/src/badges/`: every gap there is the
/// constant, never a copy of its number.
#[test]
fn ac_p4_40_10_no_dormancy_literal_in_badges() {
    let files = support::source_scan::rust_sources("src/badges");
    let value = i128::from(REVIVAL_GAP_DAYS);
    let mut literals = 0;
    let mut hits = Vec::new();
    for (name, source) in &files {
        let found = support::source_scan::integer_literals(source);
        literals += found.len();
        if found.contains(&value) {
            hits.push(name.clone());
        }
    }
    eprintln!(
        "{} files under core/src/badges/, {literals} integer literals, {value} in {hits:?}",
        files.len()
    );
    assert!(!files.is_empty(), "the scan read no file");
    assert!(literals > 0, "the scan read no integer literal");
    assert!(
        hits.is_empty(),
        "{value} is spelled as a literal in {hits:?}"
    );
}

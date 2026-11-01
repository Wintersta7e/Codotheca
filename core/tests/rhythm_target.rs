//! §39.8's first settlement: over the 52 complete weeks before it, the target is the lower median
//! of the active weeks, or the fallback below four of them; a fallback is replaced once, a chosen
//! target never, and the pass writes nothing before first run ends or while history is pending.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use codotheca_core::clock::local_date;
use codotheca_core::protocol::RhythmTargetBasis;
use codotheca_core::rings::rhythm::{
    best_weeks, load_weeks, run_weeks, settle, RhythmVerdict, SettleReport,
};
use codotheca_core::rings::targets::{rhythm_target, KEY_RHYTHM_TARGET, KEY_RHYTHM_TARGET_BASIS};
use codotheca_core::surfaces::{settings, SurfaceCtx};
use support::rings_rig::{Flags, RingsRig, UTC};

const DAY: i64 = 86_400;
/// Monday 2026-09-14, the first settlement's week.
const ARRIVAL: i64 = 20_710;
/// Wednesday noon of that week.
const NOW: i64 = (ARRIVAL + 2) * DAY + 12 * 3_600;

/// The Monday `k` weeks before the first settlement's week (negative: after it).
const fn week(k: i64) -> i64 {
    ARRIVAL - 7 * k
}

/// Commit-days on the first `days` days of each `(weeks back, days)`, by a project with no copy
/// — so nothing is pending.
fn history(rig: &RingsRig, weeks: &[(i64, i64)]) {
    let author = rig.project(Flags::default());
    for &(k, days) in weeks {
        for d in 0..days {
            rig.commit_day(author, week(k) + d);
        }
    }
}

/// A library whose first run has ended, holding `weeks` of history.
fn library(weeks: &[(i64, i64)]) -> RingsRig {
    let rig = RingsRig::scanned();
    rig.meta("first_run_completed_at", "1");
    history(&rig, weeks);
    rig
}

fn settle_at(rig: &mut RingsRig, now: i64, mark: &mut i64) -> SettleReport {
    rig.tmp
        .index_mut()
        .with_tx(|tx| settle(tx, now, &UTC, mark))
        .unwrap()
}

/// `(week_start, target, verdict)` of every row, oldest first.
fn rows(rig: &RingsRig) -> Vec<(String, u32, RhythmVerdict)> {
    load_weeks(rig.conn())
        .unwrap()
        .into_iter()
        .map(|w| (w.week_start, w.target, w.verdict))
        .collect()
}

fn max_event_id(rig: &RingsRig) -> i64 {
    rig.conn()
        .query_row("SELECT COALESCE(MAX(id), 0) FROM xp_events", [], |r| {
            r.get(0)
        })
        .unwrap()
}

fn steady(weeks: std::ops::RangeInclusive<i64>, days: i64) -> Vec<(i64, i64)> {
    weeks.map(|k| (k, days)).collect()
}

/// One fixture history and the target its first settlement must arrive at.
struct Case {
    name: &'static str,
    weeks: Vec<(i64, i64)>,
    chosen: Option<u32>,
    want: (u32, RhythmTargetBasis),
}

fn cases() -> Vec<Case> {
    let mut beyond = vec![(53, 7)];
    beyond.extend([(1, 2), (2, 2), (3, 2)]);
    vec![
        Case {
            name: "three days in twenty weeks derives 3, not 0",
            weeks: steady(1..=20, 3),
            chosen: None,
            want: (3, RhythmTargetBasis::Derived),
        },
        Case {
            name: "a commit-day 53 weeks back is ignored: three active weeks, the fallback",
            weeks: beyond,
            chosen: None,
            want: (3, RhythmTargetBasis::Fallback),
        },
        Case {
            name: "an even count takes the lower middle: 1 2 5 6 derives 2",
            weeks: vec![(1, 1), (2, 2), (3, 5), (4, 6)],
            chosen: None,
            want: (2, RhythmTargetBasis::Derived),
        },
        Case {
            name: "three active weeks: the fallback, 3",
            weeks: vec![(1, 5), (2, 5), (3, 5)],
            chosen: None,
            want: (3, RhythmTargetBasis::Fallback),
        },
        Case {
            name: "a chosen 5 is kept over a derivable history",
            weeks: steady(1..=20, 3),
            chosen: Some(5),
            want: (5, RhythmTargetBasis::Chosen),
        },
    ]
}

fn first_settlements() {
    eprintln!("case | report | target");
    for case in cases() {
        let mut rig = library(&case.weeks);
        if let Some(chosen) = case.chosen {
            rig.meta(KEY_RHYTHM_TARGET, &chosen.to_string());
            rig.meta(KEY_RHYTHM_TARGET_BASIS, "chosen");
        }
        let mut mark = 0;
        let report = settle_at(&mut rig, NOW, &mut mark);
        let target = rhythm_target(rig.conn()).unwrap();
        eprintln!("{} | {report:?} | {target:?}", case.name);
        assert!(report.first_settlement, "{}", case.name);
        assert_eq!(target, Some(case.want), "{}", case.name);
        let settled = rows(&rig);
        assert_eq!(settled.len(), 52, "{}: the window", case.name);
        assert_eq!(settled[0].0, local_date(week(52)), "{}", case.name);
        assert_eq!(settled[51].0, local_date(week(1)), "{}", case.name);
        assert!(
            settled.iter().all(|r| r.1 == case.want.0),
            "{}: every week settles against the target",
            case.name
        );
        assert_eq!(mark, max_event_id(&rig), "{}: the raise mark", case.name);
    }
}

/// Three one-day weeks arrive at the fallback; a fourth replaces it once by `min(3, derived)`
/// after that pass's insert; five seven-day weeks afterwards change nothing.
fn the_fallback_is_replaced_once() {
    let mut rig = library(&[(1, 1), (2, 1), (3, 1)]);
    let mut mark = 0;
    settle_at(&mut rig, NOW, &mut mark);
    assert_eq!(
        rhythm_target(rig.conn()).unwrap(),
        Some((3, RhythmTargetBasis::Fallback))
    );

    history(&rig, &[(0, 1)]);
    let report = settle_at(&mut rig, NOW + 7 * DAY, &mut mark);
    let target = rhythm_target(rig.conn()).unwrap();
    eprintln!("a fourth active week | {report:?} | {target:?}");
    assert!(report.target_replaced);
    assert!(!report.first_settlement);
    assert_eq!(target, Some((1, RhythmTargetBasis::Derived)));
    let fourth = rows(&rig).pop().unwrap();
    assert_eq!(
        fourth,
        (local_date(week(0)), 3, RhythmVerdict::Missed),
        "the week the replacing pass inserted keeps the target it settled under"
    );

    history(&rig, &steady(-5..=-1, 7));
    let richer = settle_at(&mut rig, NOW + 7 * 7 * DAY, &mut mark);
    let kept = rhythm_target(rig.conn()).unwrap();
    eprintln!("a richer history afterwards | {richer:?} | {kept:?}");
    assert_eq!(richer.inserted, 6);
    assert!(!richer.target_replaced);
    assert_eq!(kept, Some((1, RhythmTargetBasis::Derived)));
}

/// A `settings.set` after the first settlement moves no settled row; the next week settles
/// against the choice.
fn a_settled_week_keeps_its_target() {
    let mut rig = library(&steady(1..=20, 3));
    let mut mark = 0;
    settle_at(&mut rig, NOW, &mut mark);
    let before = rows(&rig);
    let ctx = SurfaceCtx {
        index: rig.tmp.index(),
        now: NOW,
    };
    settings::handle_set(&ctx, serde_json::json!({ "patch": { "rhythmTarget": 6 } })).unwrap();
    assert_eq!(rows(&rig), before, "a target change moved a settled week");

    history(&rig, &[(0, 3)]);
    settle_at(&mut rig, NOW + 7 * DAY, &mut mark);
    let after = rows(&rig);
    eprintln!("after the change: {:?}", after.last());
    assert_eq!(&after[..52], &before[..]);
    assert_eq!(
        after[52],
        (local_date(week(0)), 6, RhythmVerdict::Missed),
        "the week after the choice settles against it"
    );
    assert_eq!(
        rhythm_target(rig.conn()).unwrap(),
        Some((6, RhythmTargetBasis::Chosen))
    );
}

/// Before first run ends, and while a project's history is pending, the pass writes nothing.
fn no_first_settlement_before_the_library_is_known() {
    let mut not_finished = RingsRig::scanned();
    history(&not_finished, &steady(1..=20, 3));
    let mut pending = library(&steady(1..=20, 3));
    let unread = pending.project(Flags::default());
    pending.location(unread, "present", None);
    for (name, rig) in [
        ("first_run_completed_at NULL", &mut not_finished),
        ("pendingProjects > 0", &mut pending),
    ] {
        let before = rig.conn().total_changes();
        let mut mark = 0;
        let report = settle_at(rig, NOW, &mut mark);
        eprintln!("{name} | {report:?}");
        assert_eq!(report, SettleReport::default(), "{name}");
        assert!(rows(rig).is_empty(), "{name}: a row was written");
        assert_eq!(rhythm_target(rig.conn()).unwrap(), None, "{name}");
        assert_eq!(rig.conn().total_changes(), before, "{name}: the pass wrote");
    }
}

/// A year of steady three-day weeks arrives already holding a run.
fn a_steady_year_arrives_holding_a_run() {
    let mut rig = library(&steady(1..=52, 3));
    let mut mark = 0;
    settle_at(&mut rig, NOW, &mut mark);
    let weeks = load_weeks(rig.conn()).unwrap();
    let (run, best) = (run_weeks(&weeks, false), best_weeks(&weeks));
    eprintln!("a steady year: run {run}, best {best}");
    assert_eq!(run, 52);
    assert_eq!(best, 52);
}

#[test]
fn ac_p4_39_9_the_target_is_derived_once_and_never_raised() {
    first_settlements();
    the_fallback_is_replaced_once();
    a_settled_week_keeps_its_target();
    no_first_settlement_before_the_library_is_known();
    a_steady_year_arrives_holding_a_run();
}

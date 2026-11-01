//! §39.9 and §39.13: `read_rings`, the one producer of `RingsState` — day one and a day one not
//! yet stamped waive the rings rather than showing a failure; the current week's seven cells; the
//! run and best absent before the first settlement; and the read writes nothing.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use codotheca_core::clock::local_date;
use codotheca_core::protocol::{RhythmCoverage, RhythmDay, RhythmTargetBasis, RingsState};
use codotheca_core::rings::read_rings;
use codotheca_core::rings::rhythm::{load_weeks, run_weeks, settle, RhythmVerdict};
use codotheca_core::rings::targets::{KEY_RHYTHM_TARGET, KEY_RHYTHM_TARGET_BASIS};
use support::rings_rig::{Flags, RingsRig, UTC};

const DAY: i64 = 86_400;
/// Monday 2026-09-14, the current week of most fixtures.
const MONDAY: i64 = 20_710;
/// Wednesday 2026-09-16.
const TODAY: i64 = MONDAY + 2;
const DAY_ONE_KEY: &str = "motivation_day_one";
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

const fn noon(day: i64) -> i64 {
    day * DAY + 12 * 3_600
}

fn read(rig: &RingsRig, day: i64) -> RingsState {
    read_rings(rig.conn(), noon(day), &UTC).unwrap()
}

fn set_day_one(rig: &RingsRig, day: i64) {
    rig.meta(DAY_ONE_KEY, &local_date(day));
}

/// A project with a present copy and no history read: §38's `pendingProjects` is 1.
fn pending(rig: &RingsRig) {
    let unread = rig.project(Flags::default());
    rig.location(unread, "present", None);
}

/// A project with no copy — never pending — committing on each of `days`.
fn commits(rig: &RingsRig, days: &[i64]) {
    let author = rig.project(Flags::default());
    for &day in days {
        rig.commit_day(author, day);
    }
}

/// One settled row, raw, as the first settlement would have left it.
fn seed(rig: &RingsRig, week: i64, active_days: u32, verdict: &str) {
    rig.conn()
        .execute(
            "INSERT INTO rhythm_week
                (week_start, active_days, target, verdict, git_coverage, settled_at)
             VALUES (?1, ?2, 3, ?3, 'complete', 1)",
            rusqlite::params![local_date(week), active_days, verdict],
        )
        .unwrap();
    rig.meta(KEY_RHYTHM_TARGET, "3");
    rig.meta(KEY_RHYTHM_TARGET_BASIS, "derived");
}

fn settle_at(rig: &mut RingsRig, now: i64) {
    let mut mark = 0;
    rig.tmp
        .index_mut()
        .with_tx(|tx| settle(tx, now, &UTC, &mut mark))
        .unwrap();
}

fn verdict_of(rig: &RingsRig, week: i64) -> RhythmVerdict {
    load_weeks(rig.conn())
        .unwrap()
        .into_iter()
        .find(|w| w.week_start == local_date(week))
        .unwrap_or_else(|| panic!("no row for {}", local_date(week)))
        .verdict
}

fn assert_waived(state: &RingsState, name: &str) {
    assert!(state.waived, "{name}");
    assert_eq!(
        (&state.focus, &state.ship, &state.tend),
        (&None, &None, &None),
        "{name}: a waived reading carries no ring"
    );
    assert_eq!(state.rhythm.days.len(), 7, "{name}: the rhythm is present");
}

/// Day one on each weekday, read on it and every later day of its week, witnessed or not, with
/// history pending or not: the day-one cell is `active` or `waived`, never `inactive`.
fn day_one_cell_sweep() {
    let mut sweep = 0;
    for witnessed in [false, true] {
        for history_pending in [false, true] {
            let rig = RingsRig::new();
            if witnessed {
                commits(&rig, &(MONDAY..MONDAY + 7).collect::<Vec<_>>());
            }
            if history_pending {
                pending(&rig);
            }
            for first in 0..7 {
                set_day_one(&rig, MONDAY + first);
                for today in first..7 {
                    let cell =
                        read(&rig, MONDAY + today).rhythm.days[usize::try_from(first).unwrap()];
                    let want = if witnessed {
                        RhythmDay::Active
                    } else {
                        RhythmDay::Waived
                    };
                    assert_eq!(
                        cell,
                        want,
                        "day one on {} read on {} (witnessed {witnessed}, pending {history_pending})",
                        WEEKDAYS[usize::try_from(first).unwrap()],
                        WEEKDAYS[usize::try_from(today).unwrap()],
                    );
                    sweep += 1;
                }
            }
        }
    }
    eprintln!("day-one cells swept: {sweep}");
    assert_eq!(sweep, 112);
}

/// The live pass: two counted weeks, the week holding day one with no active day, then a met
/// week — the day-one week settles `waived` and the run runs across it.
fn the_live_pass_waives_the_day_one_week() {
    let mut rig = RingsRig::new();
    seed(&rig, MONDAY - 14, 3, "counted");
    seed(&rig, MONDAY - 7, 4, "counted");
    set_day_one(&rig, MONDAY + 2);
    commits(&rig, &[MONDAY + 7, MONDAY + 8, MONDAY + 9]);
    settle_at(&mut rig, noon(MONDAY + 16));
    let weeks = load_weeks(rig.conn()).unwrap();
    let run = run_weeks(&weeks, false);
    eprintln!(
        "live pass: day-one week {:?}, run {run}",
        verdict_of(&rig, MONDAY)
    );
    assert_eq!(verdict_of(&rig, MONDAY), RhythmVerdict::Waived);
    assert_eq!(verdict_of(&rig, MONDAY + 7), RhythmVerdict::Counted);
    assert_eq!(run, 3, "the waived week neither breaks nor extends the run");

    let mut met = RingsRig::new();
    seed(&met, MONDAY - 7, 3, "counted");
    set_day_one(&met, MONDAY + 2);
    commits(&met, &[MONDAY, MONDAY + 1, MONDAY + 2]);
    settle_at(&mut met, noon(MONDAY + 9));
    assert_eq!(verdict_of(&met, MONDAY), RhythmVerdict::Counted);
}

/// The first settlement, with day one inside its window — the restored-index shape: twenty
/// three-day weeks, the one holding day one empty.
fn the_first_settlement_waives_the_day_one_week() {
    let mut rig = RingsRig::new();
    rig.meta("first_run_completed_at", "1");
    let days: Vec<i64> = (1..=20)
        .filter(|&k| k != 10)
        .flat_map(|k| (0..3).map(move |d| MONDAY - 7 * k + d))
        .collect();
    commits(&rig, &days);
    set_day_one(&rig, MONDAY - 70 + 2);
    settle_at(&mut rig, noon(TODAY));
    let state = read(&rig, TODAY);
    eprintln!(
        "first settlement: day-one week {:?}, run {:?}",
        verdict_of(&rig, MONDAY - 70),
        state.rhythm.run_weeks
    );
    assert_eq!(verdict_of(&rig, MONDAY - 70), RhythmVerdict::Waived);
    assert_eq!(state.rhythm.run_weeks, Some(19), "unbroken across day one");
}

#[test]
fn ac_p4_39_7_day_one_waives_and_is_never_a_failure() {
    let rig = RingsRig::new();
    assert_waived(&read(&rig, TODAY), "day one NULL");
    set_day_one(&rig, TODAY);
    assert_waived(&read(&rig, TODAY), "day one today");
    set_day_one(&rig, TODAY - 1);
    let next = read(&rig, TODAY);
    assert!(!next.waived);
    assert!(
        next.focus.is_some() && next.ship.is_some() && next.tend.is_some(),
        "the day after day one carries all three rings: {next:?}"
    );
    day_one_cell_sweep();
    the_live_pass_waives_the_day_one_week();
    the_first_settlement_waives_the_day_one_week();
}

fn counts(rig: &RingsRig) -> (u64, i64, i64) {
    let count = |table: &str| -> i64 {
        rig.conn()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    };
    (
        rig.conn().total_changes(),
        count("app_meta"),
        count("rhythm_week"),
    )
}

/// Every fixture shape the readings above take.
fn fixtures() -> Vec<(&'static str, RingsRig)> {
    let unstamped = RingsRig::new();
    let today = RingsRig::new();
    set_day_one(&today, TODAY);
    let yesterday = RingsRig::new();
    set_day_one(&yesterday, TODAY - 1);
    commits(&yesterday, &[MONDAY, TODAY]);
    let history_pending = RingsRig::new();
    set_day_one(&history_pending, MONDAY);
    pending(&history_pending);
    let settled = RingsRig::new();
    seed(&settled, MONDAY - 7, 3, "counted");
    set_day_one(&settled, MONDAY - 7);
    let chosen = RingsRig::new();
    chosen.meta(KEY_RHYTHM_TARGET, "5");
    chosen.meta(KEY_RHYTHM_TARGET_BASIS, "chosen");
    vec![
        ("day one NULL", unstamped),
        ("day one today", today),
        ("day one yesterday, witnessed", yesterday),
        ("history pending", history_pending),
        ("a settled week", settled),
        ("a chosen target, nothing settled", chosen),
    ]
}

#[test]
fn read_rings_writes_nothing() {
    let fixtures = fixtures();
    for (name, rig) in &fixtures {
        let before = counts(rig);
        let state = read(rig, TODAY);
        assert_eq!(counts(rig), before, "{name}: read_rings wrote ({state:?})");
    }
    eprintln!("fixtures read: {}", fixtures.len());
    assert_eq!(fixtures.len(), 6);
}

#[test]
fn each_cell_state_is_reachable() {
    type Fixture = (&'static str, fn(&RingsRig), usize, RhythmDay);
    let cases: [Fixture; 6] = [
        ("Thu, after today", |_| {}, 3, RhythmDay::Future),
        (
            "Mon, a commit-day",
            |r| commits(r, &[MONDAY]),
            0,
            RhythmDay::Active,
        ),
        (
            "Wed, day one today, unwitnessed",
            |r| set_day_one(r, TODAY),
            2,
            RhythmDay::Waived,
        ),
        (
            "Wed, today, unwitnessed",
            |r| set_day_one(r, MONDAY - 7),
            2,
            RhythmDay::Pending,
        ),
        (
            "Tue, history pending",
            |r| {
                set_day_one(r, MONDAY - 7);
                pending(r);
            },
            1,
            RhythmDay::NotComputed,
        ),
        (
            "Tue, nothing witnessed, all known",
            |r| set_day_one(r, MONDAY - 7),
            1,
            RhythmDay::Inactive,
        ),
    ];
    for (name, setup, cell, want) in cases {
        let rig = RingsRig::new();
        setup(&rig);
        let days = read(&rig, TODAY).rhythm.days;
        eprintln!("{name}: {days:?}");
        assert_eq!(days[cell], want, "{name}");
    }
}

#[test]
fn git_coverage_is_partial_while_history_is_pending() {
    let rig = RingsRig::new();
    assert_eq!(
        read(&rig, TODAY).rhythm.git_coverage,
        RhythmCoverage::Complete
    );
    pending(&rig);
    let state = read(&rig, TODAY);
    eprintln!("pending: {:?}", state.rhythm);
    assert_eq!(state.rhythm.git_coverage, RhythmCoverage::Partial);
}

#[test]
fn run_and_best_are_null_before_the_first_settlement_never_zero() {
    let rig = RingsRig::new();
    commits(&rig, &[MONDAY, MONDAY + 1, TODAY]);
    let empty = read(&rig, TODAY).rhythm;
    eprintln!("nothing settled: {empty:?}");
    assert_eq!(empty.active_days, 3);
    assert_eq!(
        (
            empty.target,
            empty.target_basis,
            empty.run_weeks,
            empty.best_weeks
        ),
        (None, None, None, None)
    );
    rig.meta(KEY_RHYTHM_TARGET, "5");
    rig.meta(KEY_RHYTHM_TARGET_BASIS, "chosen");
    let chosen = read(&rig, TODAY).rhythm;
    assert_eq!(
        (
            chosen.target,
            chosen.target_basis,
            chosen.run_weeks,
            chosen.best_weeks
        ),
        (Some(5), Some(RhythmTargetBasis::Chosen), None, None)
    );
    seed(&rig, MONDAY - 7, 3, "counted");
    let settled = read(&rig, TODAY).rhythm;
    eprintln!("one settled week, three days this week: {settled:?}");
    assert_eq!((settled.run_weeks, settled.best_weeks), (Some(2), Some(1)));
}

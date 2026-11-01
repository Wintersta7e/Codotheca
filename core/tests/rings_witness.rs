//! §39.2 rule 4's week and §39.6's three witnesses, each read as a set of local dates, and their
//! union — a boolean union, never a sum.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::collections::BTreeSet;

use chrono::Datelike as _;
use codotheca_core::clock::{day_bounds, local_date, parse_local_date, week_start};
use codotheca_core::rings::active::{union_days, DaySpan};
use codotheca_core::rings::commit_days::git_days;
use codotheca_core::rings::focus::focus_days;
use codotheca_core::rings::tend::tend_days;
use codotheca_core::session::measure::{focus_seconds, focus_seconds_by_day};
use codotheca_core::testing::zone::TransitionZone;
use support::rings_rig::{Flags, RingsRig, UTC};

/// Monday 2026-09-14, as an epoch day.
const MONDAY: i64 = 20_710;
const DAY: i64 = 86_400;
const NOW: i64 = (MONDAY + 60) * DAY;

fn set(days: &[i64]) -> BTreeSet<i64> {
    days.iter().copied().collect()
}

#[test]
fn week_start_is_the_monday_on_or_before() {
    assert_eq!(week_start(0), -3, "epoch day 0 is a Thursday");
    let mut checked = 0;
    for day in MONDAY - 550..MONDAY + 550 {
        let monday = week_start(day);
        assert!(monday <= day && day - monday <= 6, "{day} → {monday}");
        assert_eq!(week_start(monday), monday, "{monday} is its own week");
        let date = chrono::NaiveDate::parse_from_str(&local_date(monday), "%Y-%m-%d").unwrap();
        assert_eq!(date.weekday(), chrono::Weekday::Mon, "{monday}");
        checked += 1;
    }
    eprintln!("days checked: {checked}");
    assert_ne!(checked, 0);
}

#[test]
fn parse_local_date_inverts_local_date() {
    let mut checked = 0;
    for day in MONDAY - 3_653..MONDAY + 3_653 {
        assert_eq!(parse_local_date(&local_date(day)), Some(day), "{day}");
        checked += 1;
    }
    for bad in [
        "2026-02-30",
        "2026-9-1",
        "",
        "2026-09-01 ",
        "+026-09-01",
        "2026/09/01",
        "2026-13-01",
    ] {
        assert_eq!(parse_local_date(bad), None, "{bad:?}");
    }
    eprintln!("dates round-tripped: {checked}");
    assert_ne!(checked, 0);
}

#[test]
fn g_counts_non_shallow_non_reference_unmerged_projects_only() {
    let rig = RingsRig::new();
    let live = rig.project(Flags::default());
    let shallow = rig.project(Flags {
        shallow: true,
        ..Flags::default()
    });
    let reference = rig.project(Flags {
        reference: true,
        ..Flags::default()
    });
    let merged = rig.project(Flags {
        merged_into: Some(live),
        ..Flags::default()
    });
    let tombstoned = rig.project(Flags {
        removed_at: Some(5),
        ..Flags::default()
    });
    for (offset, project) in [live, shallow, reference, merged, tombstoned]
        .into_iter()
        .enumerate()
    {
        rig.commit_day(project, MONDAY + i64::try_from(offset).unwrap());
    }
    let span = DaySpan {
        first: MONDAY,
        last: MONDAY + 6,
    };
    let days = git_days(rig.conn(), span).unwrap();
    eprintln!("commit days witnessed: {days:?}");
    assert_eq!(days, set(&[MONDAY, MONDAY + 4]));
    // A date outside the span is not read.
    let later = DaySpan {
        first: MONDAY + 1,
        last: MONDAY + 3,
    };
    assert_eq!(git_days(rig.conn(), later).unwrap(), set(&[]));
}

#[test]
fn s_needs_the_closed_ring_not_any_credit() {
    let rig = RingsRig::new();
    let alpha = rig.project(Flags::default());
    let beta = rig.project(Flags::default());
    let at = |day: i64| day * DAY + 9 * 3_600;
    rig.segment(alpha, at(MONDAY), 29 * 60, Some(true));
    rig.segment(alpha, at(MONDAY + 1), 30 * 60, Some(true));
    rig.segment(alpha, at(MONDAY + 2), 40 * 60, Some(false));
    rig.segment(alpha, at(MONDAY + 3), 20 * 60, Some(true));
    rig.segment(beta, at(MONDAY + 3) + 10 * 60, 20 * 60, Some(true));
    let span = DaySpan {
        first: MONDAY,
        last: MONDAY + 6,
    };
    let days = focus_days(rig.conn(), span, 30 * 60, &UTC, NOW).unwrap();
    eprintln!("focus days witnessed: {days:?}");
    assert_eq!(days, set(&[MONDAY + 1, MONDAY + 3]));
    // The overlap is 30 minutes of union, not 40 of sum: a 31-minute target closes on no day.
    let above = focus_days(rig.conn(), span, 31 * 60, &UTC, NOW).unwrap();
    assert_eq!(above, set(&[]), "two projects' overlap was summed");
}

#[test]
fn the_span_form_equals_the_per_date_measure() {
    let rig = RingsRig::new();
    let alpha = rig.project(Flags::default());
    let beta = rig.project(Flags::default());
    // A switch to a later offset on the Wednesday morning: a 23-hour day inside the span.
    let zone = TransitionZone {
        at: (MONDAY + 2) * DAY + 3_600,
        before_min: 60,
        after_min: 120,
    };
    let midnight = |day: i64| day_bounds(&zone, day).0;
    // Overlapping, across projects.
    rig.segment(alpha, midnight(MONDAY) + 3_600, 3_000, Some(true));
    rig.segment(beta, midnight(MONDAY) + 5_000, 3_000, Some(true));
    // Nested.
    rig.segment(alpha, midnight(MONDAY + 1) + 1_000, 7_200, Some(true));
    rig.segment(beta, midnight(MONDAY + 1) + 2_000, 600, Some(true));
    // Across local midnight, and across the transition.
    rig.segment(alpha, midnight(MONDAY + 2) - 1_800, 9_000, Some(true));
    // Over three dates.
    rig.segment(beta, midnight(MONDAY + 4) - 600, 2 * DAY, Some(true));
    // Uncounted, inside a counted one: changes nothing either way.
    rig.segment(alpha, midnight(MONDAY + 5) + 100, 300, Some(false));

    let (first, last) = (MONDAY - 1, MONDAY + 7);
    let whole = focus_seconds_by_day(rig.conn(), &zone, first, last, NOW).unwrap();
    assert_eq!(whole[&MONDAY], 4_400, "the overlap is one person's time");
    assert_eq!(
        whole[&(MONDAY + 1)],
        9_000, // 7 200 around a nested 600, plus the half hour before the next midnight
        "the nested segment adds nothing"
    );
    let mut checked = 0;
    let mut nonzero = 0;
    for now in [NOW, midnight(MONDAY + 4) + 3_600] {
        let by_day = focus_seconds_by_day(rig.conn(), &zone, first, last, now).unwrap();
        assert_eq!(
            by_day.keys().copied().collect::<Vec<_>>(),
            (first..=last).collect::<Vec<_>>(),
            "every date of the span is answered"
        );
        for day in first..=last {
            let single = focus_seconds(rig.conn(), &zone, day, now).unwrap();
            assert_eq!(by_day[&day], single, "day {day} at {now}");
            checked += 1;
            nonzero += usize::from(single > 0);
        }
    }
    eprintln!("dates checked: {checked}, {nonzero} with focus");
    assert_ne!(checked, 0);
    assert!(nonzero >= 5, "the fixture must put focus on most dates");
}

#[test]
fn t_counts_priced_debt_days_only() {
    let rig = RingsRig::new();
    let live = rig.project(Flags::default());
    let reference = rig.project(Flags {
        reference: true,
        ..Flags::default()
    });
    let merged = rig.project(Flags {
        merged_into: Some(live),
        ..Flags::default()
    });
    let noon = |day: i64| day * DAY + 12 * 3_600;
    rig.debt_day(live, noon(MONDAY), 0, &["abandoned_with_debt"]);
    rig.debt_day(
        live,
        noon(MONDAY + 1),
        0,
        &["abandoned_with_debt", "todo_marker"],
    );
    rig.debt_day(reference, noon(MONDAY + 2), 0, &["todo_marker"]);
    rig.debt_day(merged, noon(MONDAY + 2), 0, &["todo_marker"]);
    // Archived after the row was earned: the day stays tended.
    let archived = rig.project(Flags::default());
    rig.debt_day(archived, noon(MONDAY + 3), 0, &["missing_readme"]);
    rig.conn()
        .execute(
            "UPDATE project SET is_archived = 1 WHERE id = ?1",
            [archived.0],
        )
        .unwrap();
    // 23:00 UTC at +02:00 is the next local date; 01:00 UTC at -02:00 is the previous one.
    rig.debt_day(live, (MONDAY + 4) * DAY + 23 * 3_600, 120, &["todo_marker"]);
    rig.debt_day(live, (MONDAY + 7) * DAY + 3_600, -120, &["todo_marker"]);
    let span = DaySpan {
        first: MONDAY,
        last: MONDAY + 6,
    };
    let days = tend_days(rig.conn(), span).unwrap();
    eprintln!("tend days witnessed: {days:?}");
    assert_eq!(days, set(&[MONDAY + 1, MONDAY + 3, MONDAY + 5, MONDAY + 6]));
}

#[test]
fn the_union_counts_a_day_once() {
    let union = union_days(&set(&[1, 2]), &set(&[2, 3]), &set(&[3, 4, 1]));
    assert_eq!(union, set(&[1, 2, 3, 4]));
    assert_eq!(union.len(), 4, "a day witnessed twice counts once");
    assert_eq!(union_days(&set(&[]), &set(&[]), &set(&[])), set(&[]));
    let span = DaySpan {
        first: MONDAY,
        last: MONDAY + 6,
    };
    assert_eq!(span.days().count(), 7);
    assert_eq!(span.days().next(), Some(MONDAY));
}

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §40.6's Lifeline through the core's tick, over injected dates and real repositories: the pass
//! is J1 alone, a day is clear only when every project of **L** was read that day and reads
//! `pushed` as `pass` at its close, and `LIFELINE_DAYS` clear days in a row award the badge.
//!
//! Every test prints the dates it drove and the readings each close saw.

#![cfg(feature = "testkit")]

mod support;

use codotheca_core::badges::LIFELINE_DAYS;
use support::badges_rig::award_row;
use support::lifeline_rig::{date, LifelineRig, D0};

/// `(runStart, runEnd)` as dates.
fn span(start: i64, end: i64) -> (Option<String>, Option<String>) {
    (Some(date(start)), Some(date(end)))
}

/// What turns a case's `pushed` reading into the state it names.
type Breaker = fn(&mut LifelineRig, codotheca_core::protocol::ProjectId);

const BROKEN: (Option<String>, Option<String>) = (None, None);

/// A day the core did not run is unread, and the run it ends does not continue past it.
#[test]
fn ac_p4_40_12() {
    let mut rig = LifelineRig::new();
    let project = rig.project("kept");
    rig.tick();
    rig.after_days(1);
    rig.after_days(1);
    eprintln!("after {}: {:?}", date(rig.today()), rig.run());
    assert_eq!(rig.run_span(), span(D0, D0 + 1), "two read days run on");

    // The core never runs on D0 + 3.
    rig.after_days(2);
    eprintln!(
        "after {} (skipped {}): {:?}, reading {:?}",
        date(rig.today()),
        date(D0 + 3),
        rig.run(),
        rig.reading(project)
    );
    assert_eq!(
        rig.run_span(),
        span(D0, D0 + 2),
        "D0 + 2 was read and closed"
    );
    rig.after_days(1);
    eprintln!("after {}: {:?}", date(rig.today()), rig.run());
    assert_eq!(
        rig.run_span(),
        span(D0 + 4, D0 + 4),
        "the unread day restarted the run"
    );
}

/// A pass location whose read fails leaves `refstate_observed_at` on the previous day, and the
/// day it was not read on breaks the run.
#[test]
fn ac_p4_40_12_unread_location() {
    let mut rig = LifelineRig::new();
    let project = rig.project("vanishing");
    rig.tick();
    rig.after_days(1);
    assert_eq!(rig.run_span(), span(D0, D0));

    std::fs::remove_dir_all(rig.clone_of(project).path()).unwrap();
    rig.after_days(1);
    eprintln!(
        "after {}: {:?}, reading {:?}",
        date(rig.today()),
        rig.run(),
        rig.reading(project)
    );
    assert_eq!(rig.run_span(), span(D0, D0 + 1), "D0 + 1 was read");
    assert_eq!(rig.reading(project).1, Some(D0 + 1), "D0 + 2's read failed");

    rig.after_days(1);
    eprintln!("after {}: {:?}", date(rig.today()), rig.run());
    assert_eq!(rig.run_span(), BROKEN, "an unread location broke the run");
}

/// `unknown`, `na` and `fail` at the close each break the run; each is reached through the
/// product's own path and read again by a second pass the same day.
#[test]
fn ac_p4_40_12_not_clear() {
    let cases: [(&str, Breaker); 3] = [
        ("unknown", |rig, project| {
            rig.clone_of(project).git(&["branch", "--unset-upstream"]);
        }),
        ("na", |rig, project| {
            rig.handle(
                "projects.setCheckNa",
                serde_json::json!({ "id": project.0, "check": "pushed", "na": true }),
            );
        }),
        ("fail", |rig, project| rig.commit_unpushed(project)),
    ];
    for (state, make) in cases {
        let mut rig = LifelineRig::new();
        let project = rig.project(state);
        rig.tick();
        rig.after_days(1);
        assert_eq!(rig.run_span(), span(D0, D0), "{state}: D0 was clear");

        make(&mut rig, project);
        rig.restart();
        let reading = rig.reading(project);
        eprintln!("{state}: {} closes on {reading:?}", date(rig.today()));
        assert_eq!(reading.0.as_deref(), Some(state));
        assert_eq!(reading.1, Some(D0 + 1), "{state}: D0 + 1 was read");

        rig.after_days(1);
        eprintln!("{state}: after {}: {:?}", date(rig.today()), rig.run());
        assert_eq!(rig.run_span(), BROKEN, "{state} at the close broke the run");
    }
}

/// `fail` then `pass` on one day is clear: the close reads the day's last reading.
#[test]
fn ac_p4_40_12_fail_then_pass() {
    let mut rig = LifelineRig::new();
    let project = rig.project("repaired");
    rig.tick();
    rig.after_days(1);

    rig.commit_unpushed(project);
    rig.restart();
    let failed = rig.reading(project);
    rig.push(project);
    rig.restart();
    let passed = rig.reading(project);
    eprintln!("{}: {failed:?} then {passed:?}", date(rig.today()));
    assert_eq!(failed, (Some("fail".to_owned()), Some(D0 + 1)));
    assert_eq!(passed, (Some("pass".to_owned()), Some(D0 + 1)));

    rig.after_days(1);
    eprintln!("after {}: {:?}", date(rig.today()), rig.run());
    assert_eq!(
        rig.run_span(),
        span(D0, D0 + 1),
        "the last reading was a pass"
    );
}

/// An empty **L** is not clear, and the pass reads nothing for a project outside it.
#[test]
fn ac_p4_40_12_empty() {
    let mut rig = LifelineRig::new();
    let project = rig.unacknowledged("backlog");
    rig.tick();
    rig.after_days(1);
    eprintln!(
        "after {}: {:?}, jobs {:?}, reading {:?}",
        date(rig.today()),
        rig.run(),
        rig.jobs_done(),
        rig.reading(project)
    );
    assert_eq!(rig.run_span(), BROKEN, "a day over an empty L is not clear");
    assert_eq!(rig.jobs_done(), Vec::<String>::new());

    // The same library with the project in L reads, and its day closes clear.
    rig.acknowledge(project);
    rig.restart();
    rig.after_days(1);
    eprintln!("acknowledged, after {}: {:?}", date(rig.today()), rig.run());
    assert_eq!(rig.run_span(), span(D0 + 1, D0 + 1));
}

/// The stored Lifeline award.
fn held(rig: &LifelineRig) -> Option<support::badges_rig::AwardRow> {
    let guard = rig.index.lock().unwrap();
    let row = award_row(guard.conn(), "lifeline");
    drop(guard);
    row
}

/// `LIFELINE_DAYS` consecutive clear days award Lifeline at the close of the last.
#[test]
fn ac_p4_40_12_award() {
    let mut rig = LifelineRig::new();
    rig.project("steady");
    rig.tick();
    for _ in 1..LIFELINE_DAYS {
        rig.after_days(1);
    }
    eprintln!("after {}: {:?}", date(rig.today()), rig.run());
    assert_eq!(rig.run_span(), span(D0, D0 + LIFELINE_DAYS - 2));
    assert_eq!(held(&rig), None, "one day short");

    rig.after_days(1);
    let award = held(&rig);
    eprintln!("after {}: {award:?}", date(rig.today()));
    let award = award.expect("thirty clear days awarded nothing");
    assert_eq!(award.earned_on, date(D0 + LIFELINE_DAYS - 1));
    assert_eq!(award.earned_at, (D0 + LIFELINE_DAYS - 1) * 86_400);
    assert_eq!(award.tz_offset_min, 0);
    assert_eq!(
        award.evidence,
        serde_json::json!({ "runStartOn": date(D0) })
    );
    assert_eq!(award.project_id, None);
}

/// The pass is J1 alone: its settle chains no J1.5, J2, J3, J4, J6 or J7.
#[test]
fn ac_p4_40_12_no_chain() {
    let mut rig = LifelineRig::new();
    let project = rig.project("leaf");
    rig.tick();
    let done = rig.jobs_done();
    let guard = rig.index.lock().unwrap();
    let mut st = guard
        .conn()
        .prepare("SELECT job FROM project_job_state WHERE project_id = ?1 ORDER BY job")
        .unwrap();
    let rows: Vec<String> = st
        .query_map([project.0], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    drop(st);
    drop(guard);
    eprintln!("jobs done {done:?}, job rows {rows:?}");
    assert!(!done.is_empty(), "the pass ran nothing");
    assert!(done.iter().all(|job| job == "j1"), "{done:?}");
    assert_eq!(rows, ["j1"]);
}

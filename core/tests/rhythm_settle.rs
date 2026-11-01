//! §39.7's settlement pass: every complete week after the newest row settles once, oldest first;
//! a settled week only rises; the week holding day one never settles `missed`; and an idle pass
//! writes nothing. §39.8's first settlement is stood in for by one seeded row.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::path::Path;
use std::sync::Arc;

use chrono::Datelike as _;
use codotheca_core::clock::{local_date, parse_local_date, LocalZone};
use codotheca_core::firstrun::{dispatch, FirstRunEnv};
use codotheca_core::jobs::j15_authorship::{persist, AuthorshipFacts};
use codotheca_core::protocol::{ProjectId, RhythmCoverage, RhythmTargetBasis};
use codotheca_core::rings::rhythm::{
    best_weeks, day_one, load_weeks, run_weeks, settle, RhythmVerdict, RhythmWeekRow, SettleReport,
};
use codotheca_core::rings::targets::{rhythm_target, KEY_RHYTHM_TARGET, KEY_RHYTHM_TARGET_BASIS};
use codotheca_core::testing::zone::TransitionZone;
use serde_json::json;
use support::rings_rig::{Flags, RingsRig, UTC};

const DAY: i64 = 86_400;
/// Monday 2026-09-14 — the seeded row's week — and the three weeks the pass must settle.
const W0: i64 = 20_710;
const W1: i64 = W0 + 7;
const W2: i64 = W0 + 14;
const W3: i64 = W0 + 21;
/// Wednesday noon of the week after `W3`: `W3` is the last complete week.
const NOW: i64 = (W0 + 30) * DAY + 12 * 3_600;
const DAY_ONE_KEY: &str = "motivation_day_one";

/// One settled row standing in for the first settlement, and the target it settled under.
fn seed(rig: &RingsRig, week: i64, active_days: u32, target: u32) {
    let verdict = if active_days >= target {
        "counted"
    } else {
        "missed"
    };
    rig.conn()
        .execute(
            "INSERT INTO rhythm_week
                (week_start, active_days, target, verdict, git_coverage, settled_at)
             VALUES (?1, ?2, ?3, ?4, 'complete', 1)",
            rusqlite::params![local_date(week), active_days, target, verdict],
        )
        .unwrap();
    rig.meta(KEY_RHYTHM_TARGET, &target.to_string());
    rig.meta(KEY_RHYTHM_TARGET_BASIS, "derived");
}

fn settle_in(rig: &mut RingsRig, now: i64, zone: &dyn LocalZone, mark: &mut i64) -> SettleReport {
    rig.tmp
        .index_mut()
        .with_tx(|tx| settle(tx, now, zone, mark))
        .unwrap()
}

fn settle_at(rig: &mut RingsRig, now: i64, mark: &mut i64) -> SettleReport {
    settle_in(rig, now, &UTC, mark)
}

/// Every row as stored, one text line each, for a byte-for-byte comparison.
fn dump(rig: &RingsRig) -> Vec<String> {
    rig.conn()
        .prepare(
            "SELECT week_start || '|' || active_days || '|' || target || '|' || verdict || '|'
                    || git_coverage || '|' || settled_at
               FROM rhythm_week ORDER BY week_start",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// `(active_days, verdict, coverage)` of the row for `week`.
fn row_of(rig: &RingsRig, week: i64) -> (u32, RhythmVerdict, RhythmCoverage) {
    let row = load_weeks(rig.conn())
        .unwrap()
        .into_iter()
        .find(|w| w.week_start == local_date(week))
        .unwrap_or_else(|| panic!("no row for {}", local_date(week)));
    (row.active_days, row.verdict, row.git_coverage)
}

fn job(rig: &RingsRig, project: ProjectId, job: &str, state: &str) {
    rig.conn()
        .execute(
            "INSERT INTO project_job_state (project_id, job, state, at) VALUES (?1, ?2, ?3, 1)
               ON CONFLICT (project_id, job) DO UPDATE SET state = excluded.state",
            rusqlite::params![project.0, job, state],
        )
        .unwrap();
}

/// The seeded row, and commit-days on three days of `W1` and one of `W2`, by a project with
/// no copy — so nothing is pending and every settled week reads `complete`.
fn three_missing_weeks() -> RingsRig {
    let rig = RingsRig::scanned();
    seed(&rig, W0, 0, 3);
    let author = rig.project(Flags::default());
    for day in [W1, W1 + 1, W1 + 2, W2 + 4] {
        rig.commit_day(author, day);
    }
    rig
}

#[test]
fn every_complete_week_after_the_newest_row_settles_once_oldest_first() {
    let mut rig = three_missing_weeks();
    let mut mark = 0;
    let report = settle_at(&mut rig, NOW, &mut mark);
    eprintln!("first pass: {report:?}");
    assert_eq!(report.inserted, 3);
    assert!(!report.first_settlement);
    let order: Vec<String> = rig
        .conn()
        .prepare("SELECT week_start FROM rhythm_week ORDER BY rowid")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(order, [W0, W1, W2, W3].map(local_date), "oldest first");
    assert_eq!(
        row_of(&rig, W1),
        (3, RhythmVerdict::Counted, RhythmCoverage::Complete)
    );
    assert_eq!(
        row_of(&rig, W2),
        (1, RhythmVerdict::Missed, RhythmCoverage::Complete)
    );
    assert_eq!(
        row_of(&rig, W3),
        (0, RhythmVerdict::Missed, RhythmCoverage::Complete)
    );
    for week in load_weeks(rig.conn()).unwrap().iter().skip(1) {
        assert_eq!((week.target, week.settled_at), (3, NOW));
    }

    // The idle pass, and a restart's first pass, which re-reads every settled week: neither
    // writes, so neither moves the counter §43's publisher reads.
    let before = rig.conn().total_changes();
    let idle = settle_at(&mut rig, NOW + 3_600, &mut mark);
    let mut restart = 0;
    let reread = settle_at(&mut rig, NOW + 7_200, &mut restart);
    eprintln!(
        "idle: {idle:?}; restart: {reread:?}; changes {before} -> {}",
        rig.conn().total_changes()
    );
    assert_eq!(idle, SettleReport::default());
    assert_eq!(reread, SettleReport::default());
    assert_eq!(rig.conn().total_changes(), before, "an idle pass wrote");
}

#[test]
fn a_lost_target_is_restored_as_the_fallback_before_a_week_is_inserted() {
    let mut rig = three_missing_weeks();
    rig.conn()
        .execute(
            "DELETE FROM app_meta WHERE k IN (?1, ?2)",
            [KEY_RHYTHM_TARGET, KEY_RHYTHM_TARGET_BASIS],
        )
        .unwrap();
    let mut mark = 0;
    settle_at(&mut rig, NOW, &mut mark);
    assert_eq!(
        rhythm_target(rig.conn()).unwrap(),
        Some((3, RhythmTargetBasis::Fallback))
    );
    assert_eq!(row_of(&rig, W1).1, RhythmVerdict::Counted);
}

/// Three witnesses in `W1` — one day twice — and git alone in `W2`, a week the app never ran.
fn three_witnesses() -> RingsRig {
    let rig = RingsRig::scanned();
    seed(&rig, W0, 0, 3);
    let project = rig.project(Flags::default());
    rig.commit_day(project, W1);
    rig.commit_day(project, W1 + 1);
    rig.segment(project, (W1 + 1) * DAY + 9 * 3_600, 50 * 60, Some(true));
    rig.debt_day(project, (W1 + 2) * DAY + 12 * 3_600, 0, &["missing_readme"]);
    rig.commit_day(project, W2);
    rig.commit_day(project, W2 + 3);
    rig
}

#[test]
fn a_week_the_app_never_ran_settles_from_git_alone() {
    let mut rig = three_witnesses();
    let mut mark = 0;
    settle_at(&mut rig, NOW, &mut mark);
    assert_eq!(row_of(&rig, W1).0, 3, "git, focus and tend: a union");
    assert_eq!(
        row_of(&rig, W2),
        (2, RhythmVerdict::Missed, RhythmCoverage::Complete),
        "git alone settles the week"
    );
    assert_eq!(row_of(&rig, W3).0, 0);
}

#[test]
fn the_week_holding_day_one_settles_counted_or_waived_never_missed() {
    // (case, day one as stored, W1's active days, W1's verdict, W2's verdict)
    let cases: [(&str, Option<String>, i64, RhythmVerdict, RhythmVerdict); 5] = [
        (
            "day one in W1, met",
            Some(local_date(W1 + 2)),
            3,
            RhythmVerdict::Counted,
            RhythmVerdict::Missed,
        ),
        (
            "day one in W1, unmet",
            Some(local_date(W1 + 2)),
            1,
            RhythmVerdict::Waived,
            RhythmVerdict::Missed,
        ),
        (
            "day one in W2",
            Some(local_date(W2 + 6)),
            1,
            RhythmVerdict::Missed,
            RhythmVerdict::Waived,
        ),
        (
            "malformed day one",
            Some("2026-13-40".to_owned()),
            1,
            RhythmVerdict::Missed,
            RhythmVerdict::Missed,
        ),
        (
            "no day one",
            None,
            1,
            RhythmVerdict::Missed,
            RhythmVerdict::Missed,
        ),
    ];
    for (name, stored, days, w1, w2) in cases {
        let mut rig = RingsRig::scanned();
        seed(&rig, W0, 0, 3);
        let author = rig.project(Flags::default());
        for day in W1..W1 + days {
            rig.commit_day(author, day);
        }
        if let Some(text) = &stored {
            rig.meta(DAY_ONE_KEY, text);
        }
        assert_eq!(
            day_one(rig.conn()).unwrap(),
            stored.as_deref().and_then(parse_local_date),
            "{name}: day one's reading"
        );
        let mut mark = 0;
        settle_at(&mut rig, NOW, &mut mark);
        let got = (row_of(&rig, W1).1, row_of(&rig, W2).1);
        eprintln!("{name}: W1 {:?}, W2 {:?}", got.0, got.1);
        assert_eq!(got, (w1, w2), "{name}");
    }
}

#[test]
fn a_late_commit_day_raises_a_settled_week_and_nothing_lowers_one() {
    let mut rig = three_missing_weeks();
    let mut mark = 0;
    settle_at(&mut rig, NOW, &mut mark);
    assert_eq!(row_of(&rig, W2).1, RhythmVerdict::Missed);

    let late = rig.project(Flags::default());
    rig.commit_day(late, W2);
    rig.commit_day(late, W2 + 1);
    let report = settle_at(&mut rig, NOW + 60, &mut mark);
    eprintln!("raise: {report:?}");
    assert_eq!((report.inserted, report.raised), (0, 1));
    assert_eq!(
        row_of(&rig, W2),
        (3, RhythmVerdict::Counted, RhythmCoverage::Complete)
    );
    let raised = dump(&rig);
    assert!(raised.contains(&format!("{}|3|3|counted|complete|{NOW}", local_date(W2))));

    // The rows go again; a pass that re-reads every week recomputes lower and writes nothing.
    rig.conn()
        .execute("DELETE FROM xp_events WHERE project_id = ?1", [late.0])
        .unwrap();
    let mut reread = 0;
    assert_eq!(
        settle_at(&mut rig, NOW + 120, &mut reread),
        SettleReport::default()
    );
    assert_eq!(dump(&rig), raised, "a settled week lowered");
}

#[test]
fn a_partial_week_completes_when_history_is_read() {
    let mut rig = RingsRig::scanned();
    seed(&rig, W0, 0, 3);
    let project = rig.project(Flags::default());
    rig.location(project, "present", None);
    job(&rig, project, "j4", "queued");
    rig.commit_day(project, W1);
    let mut mark = 0;
    let pending = settle_at(&mut rig, NOW, &mut mark);
    assert_eq!(pending.inserted, 3);
    for week in [W1, W2, W3] {
        assert_eq!(row_of(&rig, week).2, RhythmCoverage::Partial);
    }

    job(&rig, project, "j4", "ok");
    let read = settle_at(&mut rig, NOW + 60, &mut mark);
    eprintln!("history read: {read:?}");
    assert_eq!((read.completed, read.raised, read.inserted), (3, 0, 0));
    for week in [W0, W1, W2, W3] {
        assert_eq!(row_of(&rig, week).2, RhythmCoverage::Complete);
    }
    assert_eq!(row_of(&rig, W1).0, 1);
}

// ---------------------------------------------------------------------------------------------
// AC-P4-39-8 — through production, a narrowed identity set lowers no settled week.
// ---------------------------------------------------------------------------------------------

const A: &str = "a@example.invalid";
const B: &str = "b@example.invalid";

fn env(home: &Path) -> FirstRunEnv {
    FirstRunEnv {
        sources: codotheca_core::firstrun::sources::SourceEnv {
            home: home.to_path_buf(),
            app_data: None,
            xdg_config: None,
        },
        classifier: Arc::new(codotheca_core::firstrun::classify::FixedClassifier::new(
            vec![],
        )),
        distros: Arc::new(codotheca_core::firstrun::classify::NoDistros),
        platform: codotheca_core::index::path::PathPlatform::Unix,
        skip: codotheca_core::scan::skiplist::SkipList::default(),
        cache: codotheca_core::firstrun::roots::SuggestionCache::new(),
    }
}

/// A project of one address on a present copy, its history concluded, J1.5's census written.
fn authored(rig: &mut RingsRig, email: &str) -> ProjectId {
    let project = rig.project(Flags::default());
    rig.location(project, "present", None);
    for j in ["j1", "j1_5", "j4"] {
        job(rig, project, j, "ok");
    }
    let conn = rig.tmp.index_mut().conn_mut();
    let tx = conn.transaction().unwrap();
    persist(
        &tx,
        project,
        &AuthorshipFacts {
            committers: vec![(email.to_owned(), 3)],
            authored_by_user: Some(true),
        },
    )
    .unwrap();
    tx.commit().unwrap();
    project
}

fn commit_days_of(rig: &RingsRig, project: ProjectId) -> i64 {
    rig.conn()
        .query_row(
            "SELECT count(*) FROM xp_events WHERE project_id = ?1 AND kind = 'commit_day'",
            [project.0],
            |r| r.get(0),
        )
        .unwrap()
}

fn best(rig: &RingsRig) -> u32 {
    best_weeks(&load_weeks(rig.conn()).unwrap())
}

#[test]
fn ac_p4_39_8_a_narrowed_identity_lowers_no_week() {
    let mut rig = RingsRig::scanned();
    let home = tempfile::tempdir().unwrap();
    for email in [A, B] {
        rig.conn()
            .execute(
                "INSERT INTO identity (email, is_user, source) VALUES (?1, 1, 'gitconfig')",
                [email],
            )
            .unwrap();
    }
    seed(&rig, W0, 0, 3);
    let a = authored(&mut rig, A);
    let b = authored(&mut rig, B);
    // W1 needs both authors, W2 is B's alone, W3 is A's and short of the target.
    rig.commit_day(a, W1);
    for day in [W1 + 1, W1 + 2, W2, W2 + 1, W2 + 2] {
        rig.commit_day(b, day);
    }
    rig.commit_day(a, W3);
    rig.commit_day(a, W3 + 1);

    let mut steps: Vec<(&str, u32)> = Vec::new();
    let mut mark = 0;
    settle_at(&mut rig, NOW, &mut mark);
    steps.push(("settle", best(&rig)));
    rig.commit_day(a, W3 + 2);
    settle_at(&mut rig, NOW + 60, &mut mark);
    steps.push(("raise W3", best(&rig)));
    let before_narrowing = dump(&rig);

    // The narrowing, through `identity.confirm`'s production dispatch: B is unticked, so B's
    // project moves to Reference and, its copy being present, its git rows are deleted.
    let held = commit_days_of(&rig, b);
    let answered = dispatch(
        rig.tmp.index_mut().conn_mut(),
        &env(home.path()),
        "identity.confirm",
        &json!({ "emails": [A], "apply": true }),
        NOW + 120,
    )
    .expect("identity.confirm is first run's")
    .expect("identity.confirm answered");
    assert_eq!(answered["applied"], json!(true));
    let deleted = held - commit_days_of(&rig, b);
    eprintln!("the narrowing deleted {deleted} commit_day rows");
    assert!(
        deleted > 0,
        "the narrowing deleted nothing: the test is vacuous"
    );
    steps.push(("narrow", best(&rig)));

    // A restart's first pass re-reads every settled week: W1 and W2 now recompute lower.
    let mut restart = 0;
    let report = settle_at(&mut rig, NOW + 180, &mut restart);
    steps.push(("settle", best(&rig)));
    assert_eq!(report, SettleReport::default());
    assert_eq!(dump(&rig), before_narrowing, "a narrowing lowered a week");

    for day in W0..W0 + 3 {
        rig.commit_day(a, day);
    }
    settle_at(&mut rig, NOW + 240, &mut restart);
    steps.push(("raise W0", best(&rig)));

    eprintln!("sequence (step, bestWeeks): {steps:?}");
    assert!(
        steps.windows(2).all(|w| w[1].1 >= w[0].1),
        "bestWeeks fell: {steps:?}"
    );
    assert_eq!(steps.first().map(|s| s.1), Some(2));
    assert_eq!(steps.last().map(|s| s.1), Some(4));
}

// ---------------------------------------------------------------------------------------------
// The run and best, and the shape of every row.
// ---------------------------------------------------------------------------------------------

/// Settled weeks as `(Monday, verdict)`, oldest first.
type Weeks = Vec<(i64, RhythmVerdict)>;

fn rows(weeks: &[(i64, RhythmVerdict)]) -> Vec<RhythmWeekRow> {
    weeks
        .iter()
        .map(|&(week, verdict)| RhythmWeekRow {
            week_start: local_date(week),
            active_days: if verdict == RhythmVerdict::Counted {
                3
            } else {
                1
            },
            target: 3,
            verdict,
            git_coverage: RhythmCoverage::Complete,
            settled_at: 1,
        })
        .collect()
}

#[test]
fn the_run_bridges_a_waived_week_and_breaks_at_a_missed_one() {
    use RhythmVerdict::{Counted as C, Missed as M, Waived as V};
    // (case, rows oldest first, run without and with the current week met, best)
    let cases: [(&str, Weeks, u32, u32, u32); 6] = [
        (
            "a waived week bridges",
            vec![(W0, C), (W1, M), (W2, C), (W3, V), (W3 + 7, C)],
            2,
            3,
            2,
        ),
        (
            "a missed newest row breaks",
            vec![(W0, C), (W1, C), (W2, C), (W3, M)],
            0,
            1,
            3,
        ),
        (
            "two waived weeks bridge",
            vec![(W0, C), (W1, V), (W2, V), (W3, C)],
            2,
            3,
            2,
        ),
        (
            "a calendar gap breaks",
            vec![(W0, C), (W1, C), (W3, C)],
            1,
            2,
            2,
        ),
        ("waived alone is no run", vec![(W0, V), (W1, V)], 0, 1, 0),
        ("no rows", vec![], 0, 1, 0),
    ];
    for (name, weeks, run, run_met, longest) in cases {
        let table = rows(&weeks);
        let got = (
            run_weeks(&table, false),
            run_weeks(&table, true),
            best_weeks(&table),
        );
        eprintln!("{name}: run {} / {} best {}", got.0, got.1, got.2);
        assert_eq!(got, (run, run_met, longest), "{name}");
    }
}

/// Sixty weeks of absence across a year boundary, read in a zone whose offset changes.
fn long_absence() -> RingsRig {
    let rig = RingsRig::scanned();
    seed(&rig, W0 - 7 * 60, 0, 3);
    let author = rig.project(Flags::default());
    for week in 0..60 {
        rig.commit_day(author, W0 - 7 * 60 + week * 7 + week % 7);
    }
    rig
}

#[test]
fn every_week_start_is_a_monday() {
    let zone = TransitionZone {
        at: (W0 - 200) * DAY,
        before_min: 330,
        after_min: -600,
    };
    let mut rigs = [three_missing_weeks(), three_witnesses(), long_absence()];
    let mut checked = 0;
    for rig in &mut rigs {
        let mut mark = 0;
        settle_in(rig, NOW, &zone, &mut mark);
        for week in load_weeks(rig.conn()).unwrap() {
            let date = chrono::NaiveDate::parse_from_str(&week.week_start, "%Y-%m-%d").unwrap();
            assert_eq!(date.weekday(), chrono::Weekday::Mon, "{}", week.week_start);
            checked += 1;
        }
    }
    eprintln!("rows checked: {checked}");
    assert!(checked > 60, "too few rows were settled to check");
}

#[test]
fn quiet_mode_is_never_read() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/rings");
    let mut scanned = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(!text.is_empty(), "{} read empty", path.display());
            assert!(
                !text.contains("motivation_shown"),
                "{} reads quiet mode",
                path.display()
            );
            scanned += 1;
        }
    }
    eprintln!("files scanned under src/rings: {scanned}");
    assert!(scanned > 0, "the scan read nothing");
}

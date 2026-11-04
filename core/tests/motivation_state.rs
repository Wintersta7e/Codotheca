//! §43.3–§43.5: the band's one payload, the line's queue and its reason, read by the core over a
//! real migrated index. These are unit tests of the readers and plant rows directly; the handlers
//! that call them are tested through the protocol.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use codotheca_core::clock::local_date;
use codotheca_core::motivation::keys::{
    day_one, is_day_one, level_shown, write_level_shown, KEY_BADGE_LINE_ACKED,
    KEY_LEVEL_LINE_RETURN, KEY_LEVEL_LINE_SINCE, KEY_LEVEL_SHOWN, KEY_MOTIVATION_DAY_ONE,
    KEY_MOTIVATION_SHOWN,
};
use codotheca_core::motivation::state::motivation_state;
use codotheca_core::protocol::{
    BadgeEvidence, BadgeKind, LevelState, MotivationLine, MotivationLineForm, MotivationReason,
    MotivationState, ProjectId, XpTerm,
};
use support::rings_rig::{Flags, RingsRig, UTC};

const DAY: i64 = 86_400;
/// Wednesday 2026-09-16.
const TODAY: i64 = 20_712;
const NOW: i64 = TODAY * DAY + 12 * 3_600;

fn read(rig: &mut RingsRig) -> MotivationState {
    rig.tmp
        .index_mut()
        .with_tx(|tx| motivation_state(tx, NOW, &UTC))
        .unwrap()
}

fn line(rig: &mut RingsRig) -> Option<MotivationLine> {
    read(rig).pending_line
}

/// One `xp_events` row of `kind` on local day `day`, its `meta` as given; answers its id.
fn row(rig: &RingsRig, kind: &str, project: ProjectId, day: i64, meta: &serde_json::Value) -> i64 {
    let track = if kind == "commit_day" || kind == "release" {
        "git"
    } else {
        "session"
    };
    rig.conn()
        .execute(
            "INSERT INTO xp_events
                (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key, track, meta)
             VALUES (?1, 0, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                day * DAY,
                project.0,
                format!("subject-{}", project.0),
                kind,
                format!("{kind}:{}:{day}", project.0),
                track,
                meta.to_string(),
            ],
        )
        .unwrap();
    rig.conn().last_insert_rowid()
}

/// A `commit_day` as J4 writes one: its receipt keeps only `when`.
fn commit_day(rig: &RingsRig, project: ProjectId, day: i64) -> i64 {
    row(
        rig,
        "commit_day",
        project,
        day,
        &serde_json::json!({ "when": local_date(day) }),
    )
}

fn rename(rig: &RingsRig, project: ProjectId, name: &str) {
    rig.conn()
        .execute(
            "UPDATE project SET name = ?1 WHERE id = ?2",
            rusqlite::params![name, project.0],
        )
        .unwrap();
}

/// A library at level 2 that the user was shown at level 1, with every row so far behind the
/// mark: the next row written is the first the reason reads. Answers the project.
fn shown_at_one(rig: &RingsRig) -> ProjectId {
    let old = rig.project(Flags::default());
    // Thirteen recent distinct dates at ten each: 130 XP, past `T(2)`, and no dormancy before
    // a commit-day this month.
    for n in 0..13 {
        commit_day(rig, old, TODAY - 30 + n);
    }
    rig.meta(KEY_LEVEL_SHOWN, "1");
    mark(rig);
    old
}

/// Puts every row written so far behind the reason's mark.
fn mark(rig: &RingsRig) {
    let mark: i64 = rig
        .conn()
        .query_row("SELECT MAX(id) FROM xp_events", [], |r| r.get(0))
        .unwrap();
    rig.meta(KEY_LEVEL_LINE_SINCE, &mark.to_string());
}

fn rise(line: Option<MotivationLine>) -> MotivationReason {
    let line = line.expect("a rise is pending");
    assert_eq!(line.form, MotivationLineForm::Rise);
    assert!(line.level.is_some_and(|l| l >= 2), "{line:?}");
    assert!(line.award.is_none());
    line.reason.expect("the rise names a reason")
}

/// An award of `kind`, earned and awarded on `day`, its landing as given.
fn award(rig: &RingsRig, kind: &str, day: i64, landed: Option<i64>) {
    let evidence = serde_json::to_string(&BadgeEvidence {
        project_name: Some("p1".to_owned()),
        gap_days: None,
        year: None,
        languages: None,
        credited_seconds: None,
        projects: None,
        run_start_on: None,
    })
    .unwrap();
    rig.conn()
        .execute(
            "INSERT INTO badge_award
                (kind, earned_at, tz_offset_min, earned_on, evidence, awarded_at, awarded_on,
                 landed_at, landed_on)
             VALUES (?1, ?2, 0, ?3, ?4, ?2, ?3, ?5, ?6)",
            rusqlite::params![
                kind,
                day * DAY,
                local_date(day),
                evidence,
                landed.map(|d| d * DAY + 3_600),
                landed.map(local_date),
            ],
        )
        .unwrap();
}

/// Day one three days back, and the level already shown: no level line, so a badge may show.
fn badge_ready(rig: &RingsRig) {
    rig.meta(KEY_MOTIVATION_DAY_ONE, &local_date(TODAY - 3));
    rig.meta(KEY_LEVEL_SHOWN, "1");
    rig.meta(KEY_LEVEL_LINE_SINCE, "0");
}

#[test]
fn a_new_install_reads_not_computed_with_no_figure_and_no_line() {
    let mut rig = RingsRig::new();
    let unread = rig.project(Flags::default());
    rig.location(unread, "present", None);
    let state = read(&mut rig);
    assert!(state.shown);
    let level = state.level.expect("the level is read while shown");
    assert_eq!(level.state, LevelState::NotComputed);
    assert!(level.figures.is_none());
    assert_eq!(state.xp_bar, None);
    assert_eq!(
        state.terms, None,
        "unknown is never an empty list read as zero"
    );
    assert_eq!(state.pending_line, None);
    assert!(state.rings.is_some());
}

#[test]
fn a_floored_level_carries_no_bar() {
    let mut rig = RingsRig::new();
    rig.conn()
        .execute(
            "UPDATE level_floor SET level = 5, raised_at = 1 WHERE id = 1",
            [],
        )
        .unwrap();
    let state = read(&mut rig);
    let figures = state.level.unwrap().figures.unwrap();
    assert!(figures.floored, "{figures:?}");
    assert_eq!(state.xp_bar, None);
}

#[test]
fn the_bar_is_the_fraction_of_the_current_level_and_the_terms_sum_to_the_xp() {
    let mut rig = RingsRig::new();
    shown_at_one(&rig);
    let state = read(&mut rig);
    let figures = state.level.unwrap().figures.unwrap();
    let terms = state.terms.expect("terms are read once figures exist");
    let sum: i64 = terms.iter().map(|t| t.xp).sum();
    eprintln!("terms sum {sum}, figures.xp {}", figures.xp);
    assert_eq!(sum, figures.xp);
    assert_eq!(terms.len(), XpTerm::ALL.len());
    let bar = state.xp_bar.expect("a computed, unfloored level has a bar");
    assert!((0.0..1.0).contains(&bar), "{bar}");
    assert!(
        bar > 0.0,
        "130 XP is past T(2), so the bar has moved: {bar}"
    );
}

#[test]
fn a_level_never_shown_arrives() {
    let mut rig = RingsRig::new();
    let p = rig.project(Flags::default());
    commit_day(&rig, p, TODAY - 1);
    let line = line(&mut rig).expect("an arrival is pending");
    assert_eq!(line.form, MotivationLineForm::Arrival);
    assert_eq!(line.level, Some(1));
    assert_eq!(line.reason, None);
    assert_eq!(line.award, None);
}

#[test]
fn two_forward_rows_are_one_rise_counting_both() {
    let mut rig = RingsRig::new();
    let p = shown_at_one(&rig);
    commit_day(&rig, p, TODAY - 2);
    commit_day(&rig, p, TODAY - 1);
    let reason = rise(line(&mut rig));
    assert_eq!(
        reason,
        MotivationReason {
            transitions: 2,
            term: None,
            project_name: None,
            on: None,
        }
    );
}

#[test]
fn a_commit_day_names_the_project_as_it_is_now() {
    let mut rig = RingsRig::new();
    let p = shown_at_one(&rig);
    commit_day(&rig, p, TODAY - 1);
    rename(&rig, p, "renamed");
    let reason = rise(line(&mut rig));
    assert_eq!(
        reason,
        MotivationReason {
            transitions: 1,
            term: Some(XpTerm::CommitDay),
            project_name: Some("renamed".to_owned()),
            on: Some(local_date(TODAY - 1)),
        }
    );
}

#[test]
fn a_debt_day_names_the_project_as_its_receipt_recorded_it() {
    let mut rig = RingsRig::new();
    let p = shown_at_one(&rig);
    row(
        &rig,
        "debt_day",
        p,
        TODAY - 1,
        &serde_json::json!({
            "when": local_date(TODAY - 1),
            "project_name": "as written",
            "sources": ["lint"],
        }),
    );
    rename(&rig, p, "renamed");
    let reason = rise(line(&mut rig));
    assert_eq!(
        reason,
        MotivationReason {
            transitions: 1,
            term: Some(XpTerm::DebtDay),
            project_name: Some("as written".to_owned()),
            on: Some(local_date(TODAY - 1)),
        }
    );
}

#[test]
fn a_commit_day_ending_a_dormancy_is_a_revival() {
    let mut rig = RingsRig::new();
    shown_at_one(&rig);
    let dormant = rig.project(Flags::default());
    commit_day(&rig, dormant, TODAY - 201);
    mark(&rig);
    // Two hundred days after its last commit-day is a revival.
    commit_day(&rig, dormant, TODAY - 1);
    assert_eq!(rise(line(&mut rig)).term, Some(XpTerm::Revival));
}

#[test]
fn a_commit_day_dating_a_language_first_is_a_language() {
    let mut rig = RingsRig::new();
    shown_at_one(&rig);
    let fresh = rig.project(Flags::default());
    rig.conn()
        .execute(
            "UPDATE project SET primary_language = 'Rust' WHERE id = ?1",
            [fresh.0],
        )
        .unwrap();
    commit_day(&rig, fresh, TODAY - 1);
    assert_eq!(rise(line(&mut rig)).term, Some(XpTerm::Language));
}

#[test]
fn a_silent_or_backfilled_row_is_not_counted() {
    for flag in ["silent", "backfilled"] {
        let mut rig = RingsRig::new();
        let p = shown_at_one(&rig);
        commit_day(&rig, p, TODAY - 2);
        row(
            &rig,
            "commit_day",
            p,
            TODAY - 1,
            &serde_json::json!({ "when": local_date(TODAY - 1), flag: true }),
        );
        let reason = rise(line(&mut rig));
        assert_eq!(reason.transitions, 1, "a {flag} row was counted");
        assert_eq!(reason.on, Some(local_date(TODAY - 2)));
    }
}

#[test]
fn a_rise_pending_when_quiet_ended_returns_with_no_reason() {
    let mut rig = RingsRig::new();
    let p = shown_at_one(&rig);
    commit_day(&rig, p, TODAY - 1);
    rig.meta(KEY_LEVEL_LINE_RETURN, "2");
    let line = line(&mut rig).unwrap();
    assert_eq!(line.form, MotivationLineForm::Return);
    assert_eq!(line.level, Some(2));
    assert_eq!(line.reason, None);
}

#[test]
fn a_due_award_with_no_level_line_is_a_badge_not_yet_landed() {
    let mut rig = RingsRig::new();
    badge_ready(&rig);
    award(&rig, "curator", TODAY - 1, None);
    let line = line(&mut rig).expect("the due award is a line");
    assert_eq!(line.form, MotivationLineForm::Badge);
    assert_eq!(line.level, None);
    let award = line.award.unwrap();
    assert_eq!(award.kind, BadgeKind::Curator);
    assert!(!award.landed);
}

#[test]
fn a_rise_replaces_a_due_badge() {
    let mut rig = RingsRig::new();
    let p = shown_at_one(&rig);
    rig.meta(KEY_MOTIVATION_DAY_ONE, &local_date(TODAY - 3));
    award(&rig, "curator", TODAY - 1, None);
    commit_day(&rig, p, TODAY - 1);
    assert_eq!(line(&mut rig).unwrap().form, MotivationLineForm::Rise);
}

#[test]
fn a_landed_badge_stays_until_its_click_is_acknowledged() {
    let mut rig = RingsRig::new();
    badge_ready(&rig);
    award(&rig, "curator", TODAY - 1, Some(TODAY));
    let line = line(&mut rig).expect("a landed, unclicked badge stays");
    assert_eq!(line.form, MotivationLineForm::Badge);
    let landed = line.award.unwrap();
    assert_eq!(landed.kind, BadgeKind::Curator);
    assert!(landed.landed);

    rig.meta(KEY_BADGE_LINE_ACKED, "curator");
    assert_eq!(read(&mut rig).pending_line, None);
}

#[test]
fn quiet_strips_every_field_but_shown() {
    let mut rig = RingsRig::new();
    let p = shown_at_one(&rig);
    commit_day(&rig, p, TODAY - 1);
    rig.meta(KEY_MOTIVATION_SHOWN, "0");
    let state = read(&mut rig);
    assert!(!state.shown);

    let schema: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../protocol/schema/protocol.json"
        ))
        .unwrap(),
    )
    .unwrap();
    let fields = schema["types"]["MotivationState"]["fields"]
        .as_object()
        .expect("the schema declares MotivationState's fields");
    let wire = serde_json::to_value(&state).unwrap();
    let wire = wire.as_object().unwrap();
    let mut declared: Vec<&String> = fields.keys().collect();
    let mut sent: Vec<&String> = wire.keys().collect();
    declared.sort();
    sent.sort();
    eprintln!("declared {declared:?}");
    assert_eq!(sent, declared, "every declared field is on the wire");
    for key in declared.iter().filter(|k| k.as_str() != "shown") {
        assert_eq!(wire[key.as_str()], serde_json::Value::Null, "{key}");
    }
}

#[test]
fn the_level_shown_its_mark_and_the_return_are_written_together() {
    let mut rig = RingsRig::new();
    let p = rig.project(Flags::default());
    commit_day(&rig, p, TODAY - 2);
    let last = commit_day(&rig, p, TODAY - 1);
    rig.meta(KEY_LEVEL_LINE_RETURN, "2");
    rig.tmp
        .index_mut()
        .with_tx(|tx| write_level_shown(tx, 3))
        .unwrap();
    let conn = rig.conn();
    assert_eq!(level_shown(conn).unwrap(), Some(3));
    let meta = |k: &str| -> Option<String> {
        conn.query_row("SELECT v FROM app_meta WHERE k = ?1", [k], |r| r.get(0))
            .ok()
    };
    assert_eq!(meta(KEY_LEVEL_LINE_SINCE), Some(last.to_string()));
    assert_eq!(meta(KEY_LEVEL_LINE_RETURN), None);

    let mut empty = RingsRig::new();
    empty
        .tmp
        .index_mut()
        .with_tx(|tx| write_level_shown(tx, 1))
        .unwrap();
    let since: String = empty
        .conn()
        .query_row(
            "SELECT v FROM app_meta WHERE k = ?1",
            [KEY_LEVEL_LINE_SINCE],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(since, "0", "an empty ledger's mark is 0");
}

#[test]
fn day_one_suppresses_nothing_while_it_is_unwritten() {
    let rig = RingsRig::new();
    let today = local_date(TODAY);
    assert_eq!(day_one(rig.conn()).unwrap(), None);
    assert!(!is_day_one(rig.conn(), &today).unwrap());
    rig.meta(KEY_MOTIVATION_DAY_ONE, &today);
    assert_eq!(day_one(rig.conn()).unwrap(), Some(today.clone()));
    assert!(is_day_one(rig.conn(), &today).unwrap());
    assert!(!is_day_one(rig.conn(), &local_date(TODAY + 1)).unwrap());
}

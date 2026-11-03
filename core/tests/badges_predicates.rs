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

use codotheca_core::badges::COMEBACK_GAP_DAYS;
use codotheca_core::clock::local_date;
use codotheca_core::ledger::price::REVIVAL_GAP_DAYS;
use codotheca_core::protocol::ProjectId;
use support::badges_rig::{BadgesRig, Flags};

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
